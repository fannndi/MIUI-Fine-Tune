//! Daemon: the stdio JSON-lines service behind `miui-ft serve`.
//!
//! Layer map (read in this order):
//! - `proto.rs`         wire format (Command/Event)
//! - `config.rs`        config.json cache (app-owned file)
//! - `arbiter.rs`       pure decision table
//! - `worker.rs`        coalescing apply worker (thread)
//! - `engine_driver.rs` engine adapter (in-process, no `su`)
//! - `settings.rs`      live flags via the `settings` CLI
//! - `mod.rs`           this file: state, timers, command dispatch
//!
//! Lifecycle: spawned by the app's foreground service with the app's stdio
//! bridged. stdin EOF (app death) or `shutdown` ends the process; the app
//! never needs to kill it.

mod arbiter;
mod bridge;
mod config;
mod engine_driver;
mod proto;
mod settings;
#[cfg(test)]
mod test_util;
mod watcher;
mod worker;

pub use proto::{Command, Event, Publisher, Snapshot, VERSION};

use crate::daemon::arbiter::{ArbiterInput, Decision, SLEEP_PROFILE};
use crate::daemon::bridge::{Bridge, SyncCtx};
use crate::daemon::config::ConfigFile;
use crate::daemon::settings::LiveFlags;
use crate::daemon::watcher::{Watcher, WatcherKind};
use crate::daemon::worker::{RestoredEvent, Work};
use crate::engine::apply::Store;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SUPERVISE_MS: u64 = 3_000; // watcher health / periodic cadence
const PERIODIC_TICKS: u64 = 5; // * SUPERVISE_MS = 15 s re-evaluate cadence
const CONFIG_POLL_MS: u64 = 1_000; // mtime fallback (app also pings)
const SLEEP_DELAY_MS: u64 = 10_000; // screen off -> sleep grace period

/// Messages the main loop consumes.
enum Msg {
    Command(Command),
    Applied(worker::AppliedEvent),
    Restored(RestoredEvent),
    StdinClosed,
    /// Raw foreground event from the daemon's own logcat stream.
    Fg(String),
    /// Multi-window state from the daemon's own logcat stream.
    Mw { active: bool, other: Option<String> },
    /// Wake/unlock seed from the daemon's own peek.
    /// `unlock` always re-evaluates; `peek` (stream-down fallback) only
    /// evaluates when the package actually changed.
    Peek { pkg: Option<String>, trigger: &'static str },
    /// A watcher stream died; the supervisor restarts it with backoff.
    WatcherDown(WatcherKind),
}

/// All mutable daemon state — owned by the main loop, no locks.
struct Runtime {
    publisher: Arc<Publisher>,
    work_tx: Sender<Work>,
    /// Bridge sync requests (settings IO happens on the bridge thread).
    sync_tx: Sender<SyncCtx>,
    /// Own sender back (peek threads + watcher restarts).
    tx: Sender<Msg>,
    config: ConfigFile,
    flags: Arc<LiveFlags>,
    bridge: Arc<Bridge>,
    // device context
    screen_on: bool,
    locked: bool,
    multi_window: bool,
    second_window: Option<String>,
    /// Deduped raw foreground (the app's `lastForeground`).
    last_fg: Option<String>,
    /// Last non-transient package (the app's `lastRealPkg`).
    last_real: Option<String>,
    // decision mirror
    active: Option<String>,
    reason: Option<String>,
    src_pkg: Option<String>,
    ultra: bool,
    retired: bool,
    // timers
    sleep_deadline: Option<Instant>,
    // watchers
    fg_watcher: Option<Watcher>,
    mw_watcher: Option<Watcher>,
    last_watcher_restart: Instant,
    // event dedupe
    last_state: Snapshot,
}

impl Runtime {
    fn log(&self, msg: &str) {
        self.publisher.log(msg);
    }

    fn emit_state(&mut self, force: bool) {
        let snap = Snapshot {
            screen_on: self.screen_on,
            locked: self.locked,
            multi_window: self.multi_window,
            second_window: self.second_window.clone(),
            foreground: self.last_fg.clone(),
            active: self.active.clone(),
            reason: self.reason.clone(),
            src_pkg: self.src_pkg.clone(),
        };
        if force || snap != self.last_state {
            self.last_state = snap.clone();
            self.publisher.emit(&Event::State { state: snap });
        }
    }

    fn emit_decision(&self, trigger: &str, action: &str, profile: Option<String>, reason: Option<String>) {
        self.publisher.emit(&Event::Decision {
            trigger: trigger.into(),
            action: action.into(),
            profile,
            reason,
        });
    }

    /// The one decision point. `fg_override` mirrors the old `evaluate`:
    /// config toggles and unlocks evaluate for the *real* package even
    /// though our own (transient) app is technically in front.
    fn evaluate(&mut self, trigger: &str, fg_override: Option<String>) {
        if self.retired {
            return;
        }
        let cfg = self.config.get();
        if !cfg.enabled {
            // the app owns lifecycle: it stops the service (and thus us)
            return;
        }
        let fg = fg_override.or_else(|| self.last_fg.clone());
        let input = ArbiterInput {
            service_enabled: true,
            screen_on: self.screen_on,
            keyguard_locked: self.locked,
            foreground_pkg: fg.clone(),
            app_map: cfg.app_map.clone(),
            base_profile: cfg.base_profile.clone(),
            sleep_profile: SLEEP_PROFILE.to_string(),
            // attribution: a saver flag WE hold for a mapped app is invisible
            // to the decision — only the user's own saver forces the base
            saver_on: self.bridge.user_saver(self.flags.saver()),
            ultra_saver: self.ultra,
            multi_window: self.multi_window,
            dynamic_profile: cfg.dynamic,
        };
        match arbiter::decide(&input) {
            Decision::None => {
                self.log(&format!("evaluate({trigger}): no-op"));
                self.emit_decision(trigger, "none", None, None);
            }
            Decision::Apply { profile, reason } => {
                self.log(&format!("evaluate({trigger}): -> {profile} ({reason})"));
                self.emit_decision(trigger, "apply", Some(profile.clone()), Some(reason.clone()));
                let job = worker::Job {
                    profile,
                    reason,
                    src_pkg: fg,
                    used_saver: input.saver_on,
                    queued: Instant::now(),
                };
                let _ = self.work_tx.send(Work::Apply(job));
            }
            Decision::Retire => {
                self.log(&format!("evaluate({trigger}): MIUI ultra saver — retiring service"));
                self.emit_decision(trigger, "retire", None, None);
                self.retired = true;
                let _ = self.work_tx.send(Work::Restore { retire: true });
            }
        }
        // MIUI bridge extras: settings IO on the bridge thread (never blocks
        // the decision loop); serialized there by the bridge Mutex.
        let _ = self.sync_tx.send(SyncCtx {
            last_real: self.last_real.clone(),
            screen_on: self.screen_on,
            locked: self.locked,
            dynamic: cfg.dynamic,
            sync_perf: cfg.sync_miui_perf,
            sync_saver: cfg.sync_miui_saver,
            game_checker: cfg.game_mode_checker,
            app_map: cfg.app_map.clone(),
        });
    }

    fn on_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Hello => {
                self.log("ipc: hello");
                self.emit_state(true);
            }
            Command::Ping => self.publisher.emit(&Event::Pong),
            Command::Shutdown => {
                self.log("ipc: shutdown");
                return false;
            }
            Command::ConfigChanged => {
                if self.config.reload_if_changed() {
                    self.log("config reloaded");
                }
                let fg = self.last_real.clone().or_else(|| self.last_fg.clone());
                self.evaluate("config", fg);
            }
            Command::Screen { on, locked } => {
                let was_on = self.screen_on;
                self.screen_on = on;
                self.locked = locked;
                if !on {
                    if was_on {
                        self.log("screen off");
                    }
                    self.sleep_deadline = Some(Instant::now() + Duration::from_millis(SLEEP_DELAY_MS));
                } else {
                    self.sleep_deadline = None;
                    if !was_on {
                        self.log("screen on");
                    }
                    if !self.locked {
                        self.peek_now("unlock");
                    }
                }
                self.emit_state(false);
            }
            Command::UserPresent => {
                self.locked = false;
                self.peek_now("unlock");
                self.emit_state(false);
            }
            Command::Fg { pkg } => self.on_fg(pkg),
            Command::Seed { pkg } => self.on_seed(pkg, "unlock"),
            Command::Mw { active, other } => self.on_mw(active, other),
            Command::Ultra { on } => {
                self.ultra = on;
                self.log(&format!("MIUI extreme saver: {on}"));
                if on {
                    self.evaluate("ultra", None);
                }
            }
            Command::SetBase { profile } => {
                // the app writes config.json first; sync, then apply as base
                self.config.reload_if_changed();
                self.log(&format!("set_base: {profile}"));
                let job = worker::Job {
                    profile,
                    reason: "base".into(),
                    src_pkg: None,
                    used_saver: false,
                    queued: Instant::now(),
                };
                let _ = self.work_tx.send(Work::Apply(job));
            }
            Command::Restore => {
                self.log("ipc: restore");
                let _ = self.work_tx.send(Work::Restore { retire: false });
            }
        }
        true
    }

    /// Foreground package event (own watcher or the app's forwarder).
    fn on_fg(&mut self, pkg: String) {
        if !arbiter::is_transient(Some(&pkg)) {
            self.last_real = Some(pkg.clone());
        }
        if self.last_fg.as_deref() == Some(pkg.as_str()) {
            return;
        }
        self.last_fg = Some(pkg);
        self.emit_state(false);
        if self.screen_on && !self.locked {
            self.evaluate("event", None);
        }
    }

    /// Wake/unlock seed: peeked package (None = peek found nothing).
    /// `triggers`: "unlock" always evaluates (wake re-asserts the decision);
    /// "peek" only evaluates when the package changed (stream-down fallback).
    fn on_seed(&mut self, peeked: Option<String>, trigger: &str) {
        let best = match peeked.as_deref() {
            Some(p) if !arbiter::is_transient(Some(p)) => Some(p.to_string()),
            _ => self.last_real.clone().or_else(|| peeked.clone()),
        };
        self.log(&format!("seed: peeked={peeked:?} best={best:?}"));
        let changed = best.is_some() && best != self.last_fg;
        if let Some(b) = best {
            self.last_fg = Some(b);
        }
        self.emit_state(false);
        if trigger == "unlock" || changed {
            self.evaluate(trigger, None);
        }
    }

    /// Multi-window state change from the daemon's own watcher.
    fn on_mw(&mut self, active: bool, other: Option<String>) {
        if self.multi_window != active || self.second_window != other {
            self.multi_window = active;
            self.second_window = other.clone();
            let msg = if active {
                format!("multi-window ON ({})", other.unwrap_or_default())
            } else {
                "multi-window off".into()
            };
            self.log(&format!("bridge: {msg}"));
            self.publisher.emit(&Event::Bridge { msg });
            self.emit_state(false);
            self.evaluate("multiwindow", None);
        }
    }

    /// One-shot peek on a short thread (never blocks the main loop).
    fn peek_now(&mut self, trigger: &'static str) {
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new().name("peek".into()).spawn(move || {
            let pkg = watcher::peek_fg();
            let _ = tx.send(Msg::Peek { pkg, trigger });
        });
    }

    fn on_applied(&mut self, ev: worker::AppliedEvent) {
        if ev.ok {
            if ev.wrote == 0 {
                self.log(&format!("apply {}: in place", ev.profile));
            } else {
                self.log(&format!(
                    "apply {}: done in {}ms (settle {}ms)",
                    ev.profile, ev.ms, ev.settle_ms
                ));
            }
            self.active = Some(ev.profile.clone());
            self.reason = Some(ev.reason.clone());
            self.src_pkg = ev.src_pkg.clone();
        } else {
            self.log(&format!("apply {} failed after retry ({})", ev.profile, ev.failed));
            self.reason = Some(format!("apply failed ({})", ev.failed));
        }
        self.publisher.emit(&Event::Applied {
            profile: ev.profile,
            reason: ev.reason,
            src_pkg: ev.src_pkg,
            ok: ev.ok,
            wrote: ev.wrote,
            verified: ev.verified,
            failed: ev.failed,
            ms: ev.ms,
            settle_ms: ev.settle_ms,
        });
        self.emit_state(false);
    }

    fn on_restored(&mut self, ev: RestoredEvent) {
        if ev.ok {
            self.active = None;
            self.reason = None;
            self.src_pkg = None;
        }
        if ev.retire {
            self.log(&format!("retire: restore ok={} failed={}", ev.ok, ev.failed));
            self.publisher.emit(&Event::Retired {
                ok: ev.ok,
                wrote: ev.wrote,
                verified: ev.verified,
                failed: ev.failed,
            });
        } else {
            self.log(&format!("restore: ok={} failed={}", ev.ok, ev.failed));
            self.publisher.emit(&Event::Restored {
                ok: ev.ok,
                wrote: ev.wrote,
                verified: ev.verified,
                failed: ev.failed,
            });
        }
        self.emit_state(false);
    }
}

/// Entry point for `miui-ft serve`.
pub fn run(state_dir: &Path, config_path: &Path) -> Result<(), String> {
    let publisher = Publisher::new();
    publisher.emit(&Event::Hello { version: VERSION, pid: std::process::id() });

    let (mut cfg, cfg_err) = ConfigFile::load(config_path);
    if let Some(e) = cfg_err {
        publisher.log(&format!("config: {e}"));
    }
    cfg.reload_if_changed();

    let flags = LiveFlags::spawn();
    let bridge = Bridge::recover(state_dir, publisher.clone());
    let initial_active = Store::new(state_dir).load_state().active;

    let (msg_tx, msg_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
    let (work_tx, work_rx) = mpsc::channel::<Work>();
    let (sync_tx, sync_rx) = mpsc::channel::<SyncCtx>();

    // --- stdin reader thread -------------------------------------------------
    {
        let tx = msg_tx.clone();
        let pubr = publisher.clone();
        std::thread::Builder::new()
            .name("stdin".into())
            .spawn(move || {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Command>(&line) {
                        Ok(cmd) => {
                            if tx.send(Msg::Command(cmd)).is_err() {
                                break;
                            }
                        }
                        Err(e) => pubr.log(&format!("ipc: bad command: {e}")),
                    }
                }
                let _ = tx.send(Msg::StdinClosed);
            })
            .map_err(|e| format!("stdin thread: {e}"))?;
    }

    // --- apply worker thread -------------------------------------------------
    {
        let state_dir = state_dir.to_path_buf();
        let bridge = bridge.clone();
        let tx = msg_tx.clone();
        std::thread::Builder::new()
            .name("apply-worker".into())
            .spawn(move || {
                let mut engine = engine_driver::EngineApplier::new(&state_dir, bridge);
                let mut on_applied = |ev: worker::AppliedEvent| {
                    let _ = tx.send(Msg::Applied(ev));
                };
                let mut on_restored = |ev: RestoredEvent| {
                    let _ = tx.send(Msg::Restored(ev));
                };
                worker::run(
                    work_rx,
                    &mut engine,
                    Duration::from_millis(worker::SETTLE_MS),
                    Duration::from_millis(worker::RETRY_MS),
                    &mut on_applied,
                    &mut on_restored,
                );
            })
            .map_err(|e| format!("worker thread: {e}"))?;
    }

    // --- bridge sync thread (settings IO; never blocks decisions) ------------
    {
        let bridge = bridge.clone();
        std::thread::Builder::new()
            .name("bridge".into())
            .spawn(move || {
                // latest wins: drain pending contexts before syncing, so a
                // burst of decisions costs ONE settings read/write round.
                while let Ok(first) = sync_rx.recv() {
                    let mut latest = first;
                    while let Ok(next) = sync_rx.try_recv() {
                        latest = next;
                    }
                    bridge.sync(&latest);
                }
            })
            .map_err(|e| format!("bridge thread: {e}"))?;
    }

    // --- main loop -------------------------------------------------------------
    let mut rt = Runtime {
        publisher,
        work_tx,
        sync_tx,
        tx: msg_tx.clone(),
        config: cfg,
        flags,
        bridge,
        screen_on: true,
        locked: false,
        multi_window: false,
        second_window: None,
        last_fg: None,
        last_real: None,
        active: initial_active,
        reason: None,
        src_pkg: None,
        ultra: false,
        retired: false,
        sleep_deadline: None,
        fg_watcher: None,
        mw_watcher: None,
        last_watcher_restart: Instant::now(),
        last_state: Snapshot::default(),
    };
    rt.fg_watcher = watcher::spawn_fg(rt.tx.clone());
    rt.mw_watcher = watcher::spawn_mw(rt.tx.clone());
    if rt.fg_watcher.is_none() {
        rt.log("foreground watcher: failed to spawn logcat");
    }
    if rt.mw_watcher.is_none() {
        rt.log("multi-window watcher: failed to spawn logcat");
    }
    // initial snapshot even when everything is at defaults
    rt.emit_state(true);

    let mut next_sup = Instant::now() + Duration::from_millis(SUPERVISE_MS);
    let mut next_cfg = Instant::now() + Duration::from_millis(CONFIG_POLL_MS);
    let mut ticks: u64 = 0;

    loop {
        let now = Instant::now();
        let mut wake = next_sup.min(next_cfg);
        if let Some(s) = rt.sleep_deadline {
            wake = wake.min(s);
        }
        let timeout = wake.saturating_duration_since(now);

        match msg_rx.recv_timeout(timeout) {
            Ok(Msg::Command(cmd)) => {
                if !rt.on_command(cmd) {
                    break;
                }
            }
            Ok(Msg::Applied(ev)) => rt.on_applied(ev),
            Ok(Msg::Restored(ev)) => rt.on_restored(ev),
            Ok(Msg::Fg(pkg)) => rt.on_fg(pkg),
            Ok(Msg::Mw { active, other }) => rt.on_mw(active, other),
            Ok(Msg::Peek { pkg, trigger }) => rt.on_seed(pkg, trigger),
            Ok(Msg::WatcherDown(kind)) => {
                rt.log(&format!("{} stream ended", kind.name()));
            }
            Ok(Msg::StdinClosed) => {
                rt.log("stdin closed, exiting");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let now = Instant::now();
        if now >= next_sup {
            next_sup = now + Duration::from_millis(SUPERVISE_MS);
            ticks += 1;
            rt.supervise(ticks);
        }
        if now >= next_cfg {
            next_cfg = now + Duration::from_millis(CONFIG_POLL_MS);
            if rt.config.reload_if_changed() {
                rt.log("config reloaded (mtime)");
                let fg = rt.last_real.clone().or_else(|| rt.last_fg.clone());
                rt.evaluate("config", fg);
            }
        }
        if let Some(s) = rt.sleep_deadline {
            if now >= s {
                rt.sleep_deadline = None;
                if !rt.screen_on {
                    rt.log("sleep timer fired");
                    rt.evaluate("sleep", None);
                }
            }
        }
    }

    // stop watcher children before exit (logcat may be mid-read)
    if let Some(w) = rt.fg_watcher.as_mut() {
        w.stop();
    }
    if let Some(w) = rt.mw_watcher.as_mut() {
        w.stop();
    }
    // releases bridge holds on a clean exit; an abrupt kill self-heals next
    // start via holds.json (recover + first evaluate decides RESTORE)
    rt.bridge.release_all();
    rt.publisher.emit(&Event::Bye);
    Ok(())
}

impl Runtime {
    /// 3 s supervisor: periodic re-evaluate + watcher health + fallback peek.
    fn supervise(&mut self, ticks: u64) {
        if ticks % PERIODIC_TICKS == 0 && self.screen_on && !self.locked {
            self.evaluate("periodic", None);
        }
        // stream health
        let fg_alive = self.fg_watcher.as_ref().map(|w| w.is_alive()).unwrap_or(false);
        let mw_alive = self.mw_watcher.as_ref().map(|w| w.is_alive()).unwrap_or(false);
        if !fg_alive || !mw_alive {
            if self.last_watcher_restart.elapsed() >= watcher::RESTART_BACKOFF {
                self.last_watcher_restart = Instant::now();
                if !fg_alive {
                    self.log("foreground stream down — restarting");
                    if let Some(w) = self.fg_watcher.as_mut() {
                        w.stop();
                    }
                    self.fg_watcher = watcher::spawn_fg(self.tx.clone());
                }
                if !mw_alive {
                    self.log("multi-window stream down — restarting");
                    if let Some(w) = self.mw_watcher.as_mut() {
                        w.stop();
                    }
                    self.mw_watcher = watcher::spawn_mw(self.tx.clone());
                }
            }
            // fallback peek while the event stream is down (screen visible)
            if !fg_alive && self.screen_on && !self.locked {
                self.peek_now("peek");
            }
        }
    }
}
