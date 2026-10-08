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
mod config;
mod engine_driver;
mod proto;
mod settings;
#[cfg(test)]
mod test_util;
mod worker;

pub use proto::{Command, Event, Publisher, Snapshot, VERSION};

use crate::daemon::arbiter::{ArbiterInput, Decision, SLEEP_PROFILE};
use crate::daemon::config::ConfigFile;
use crate::daemon::settings::LiveFlags;
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
}

/// All mutable daemon state — owned by the main loop, no locks.
struct Runtime {
    publisher: Arc<Publisher>,
    work_tx: Sender<Work>,
    config: ConfigFile,
    flags: Arc<LiveFlags>,
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
            saver_on: self.flags.saver(),
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
                }
                self.emit_state(false);
            }
            Command::UserPresent => {
                self.locked = false;
                self.emit_state(false);
            }
            Command::Fg { pkg } => {
                if !arbiter::is_transient(Some(&pkg)) {
                    self.last_real = Some(pkg.clone());
                }
                if self.last_fg.as_deref() == Some(pkg.as_str()) {
                    return true;
                }
                self.last_fg = Some(pkg);
                self.emit_state(false);
                if self.screen_on && !self.locked {
                    self.evaluate("event", None);
                }
            }
            Command::Seed { pkg } => {
                let best = match pkg {
                    Some(p) if !arbiter::is_transient(Some(&p)) => Some(p),
                    _ => self.last_real.clone().or(pkg),
                };
                self.log(&format!("seed: peeked={:?} best={:?}", self.last_fg, best));
                if let Some(b) = &best {
                    self.last_fg = Some(b.clone());
                }
                self.emit_state(false);
                self.evaluate("unlock", None);
            }
            Command::Mw { active, other } => {
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
            self.publisher.emit(&Event::Retired { ok: ev.ok, failed: ev.failed });
        } else {
            self.log(&format!("restore: ok={} failed={}", ev.ok, ev.failed));
            self.publisher.emit(&Event::Restored { ok: ev.ok, failed: ev.failed });
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
    let initial_active = Store::new(state_dir).load_state().active;

    let (msg_tx, msg_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
    let (work_tx, work_rx) = mpsc::channel::<Work>();

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
        let tx = msg_tx.clone();
        std::thread::Builder::new()
            .name("apply-worker".into())
            .spawn(move || {
                let mut engine = engine_driver::EngineApplier::new(&state_dir);
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

    // --- main loop -------------------------------------------------------------
    let mut rt = Runtime {
        publisher,
        work_tx,
        config: cfg,
        flags,
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
        last_state: Snapshot::default(),
    };
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
            if ticks % PERIODIC_TICKS == 0 && rt.screen_on && !rt.locked {
                rt.evaluate("periodic", None);
            }
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

    rt.publisher.emit(&Event::Bye);
    Ok(())
}
