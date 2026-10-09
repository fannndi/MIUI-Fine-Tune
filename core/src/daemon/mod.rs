//! Daemon: the stdio JSON-lines service behind `miui-ft serve`.
//!
//! Layer map (read in this order):
//! - `proto.rs`         wire format (Command/Event)
//! - `config.rs`        config.json cache (app-owned file)
//! - `arbiter.rs`       pure decision table
//! - `runtime.rs`       main-loop state emission + applied/restored events
//! - `commands.rs`      command dispatch (IPC + watchers)
//! - `evaluate.rs`      decision point + supervisor timer
//! - `worker.rs`        coalescing apply worker (thread)
//! - `engine_driver.rs` engine adapter (in-process, no `su`)
//! - `settings.rs`      live flags via the `settings` CLI
//! - `watcher.rs`       logcat streams (foreground + multi-window)
//! - `env.rs`           environment sampler thread (read-only telemetry)
//! - `stats.rs`         transition history (bounded, persisted)
//! - `bridge/`          MIUI mode hold/restore (settings IO thread)
//! - `mod.rs`           this file: state type, constants, threads, main loop
//!
//! Lifecycle: spawned by the app's foreground service with the app's stdio
//! bridged. stdin EOF (app death) or `shutdown` ends the process; the app
//! never needs to kill it.

mod arbiter;
mod bridge;
mod commands;
mod config;
mod engine_driver;
mod env;
mod evaluate;
mod maintenance;
mod proto;
mod runtime;
mod settings;
mod stats;
#[cfg(test)]
mod test_util;
mod watcher;
mod worker;

pub use proto::{Command, Event, Publisher, Snapshot, VERSION};

/// Config shape shared with `doctor` (and future tools).
pub use config::DaemonConfig;

use crate::daemon::bridge::{Bridge, SyncCtx};
use crate::daemon::config::ConfigFile;
use crate::daemon::settings::LiveFlags;
use crate::daemon::stats::Stats;
use crate::daemon::watcher::{Watcher, WatcherKind};
use crate::daemon::worker::{RestoredEvent, Work};
use crate::engine::apply::Store;
use crate::engine::env::EnvSnapshot;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const SUPERVISE_MS: u64 = 3_000; // watcher health / periodic cadence
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
    Mw {
        active: bool,
        other: Option<String>,
    },
    /// Wake/unlock seed from the daemon's own peek.
    /// `unlock` always re-evaluates; `peek` (stream-down fallback) only
    /// evaluates when the package actually changed.
    Peek {
        pkg: Option<String>,
        trigger: &'static str,
    },
    /// Fresh environment sample (battery / thermal / GPU busy).
    Env(EnvSnapshot),
    /// Storage maintenance finished (result to persist + report).
    MaintDone(maintenance::MaintOutcome),
    /// Choreographer jank line (frames skipped) from the daemon's stream.
    Jank {
        frames: u32,
    },
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
    // paths + lifecycle (diag)
    state_dir: PathBuf,
    config_path: PathBuf,
    started: Instant,
    /// Last seen mtime of `state_dir/profiles.json` (app deploy detection).
    profiles_mtime: Option<std::time::SystemTime>,
    // environment + history
    env: EnvSnapshot,
    stats: Stats,
    /// Thermal guard hysteresis state (engage at ceiling, release below).
    thermal_stepped: bool,
    /// Last battery-guard verdict (evaluate on flip only).
    last_battery_low: bool,
    /// False until the first apply of this daemon run: the startup decision
    /// is forced so a pack update or drift that happened while we were down
    /// is reconciled.
    reconciled: bool,
    /// Auto-revive watchdog counter (3 s supervise ticks).
    watchdog_tick: u64,
    /// The app granted Do Not Disturb access (official-API bridge gate).
    dnd_granted: bool,
    /// Storage maintenance state (weekly f2fs GC while charging + idle).
    maint: maintenance::MaintFile,
    maint_running: bool,
    screen_off_since: Option<Instant>,
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
    jank_watcher: Option<Watcher>,
    last_watcher_restart: Instant,
    /// Jank-boost window (experimental FAS-lite): active until this instant.
    boost_until: Option<Instant>,
    /// Cooldown anchor (last boost start).
    last_boost: Option<Instant>,
    // event dedupe
    last_state: Snapshot,
}

/// File mtime (None when missing) — deploy/update detection.
fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Entry point for `miui-ft serve`.
pub fn run(state_dir: &Path, config_path: &Path) -> Result<(), String> {
    let publisher = Publisher::new();
    publisher.emit(&Event::Hello {
        version: VERSION,
        pid: std::process::id(),
    });
    publisher.log(&format!("daemon {} starting", env!("CARGO_PKG_VERSION")));

    // state dir first: holds/stats/state files must be writable from the start
    if let Err(e) = std::fs::create_dir_all(state_dir) {
        publisher.log(&format!("state dir: {e}"));
    }

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
        state_dir: state_dir.to_path_buf(),
        config_path: config_path.to_path_buf(),
        started: Instant::now(),
        profiles_mtime: mtime(&state_dir.join("profiles.json")),
        env: EnvSnapshot::default(),
        stats: Stats::load(state_dir),
        thermal_stepped: false,
        last_battery_low: false,
        reconciled: false,
        watchdog_tick: 0,
        dnd_granted: false,
        maint: maintenance::MaintFile::load(state_dir),
        maint_running: false,
        screen_off_since: None,
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
        jank_watcher: None,
        last_watcher_restart: Instant::now(),
        boost_until: None,
        last_boost: None,
        last_state: Snapshot::default(),
    };
    rt.fg_watcher = watcher::spawn_fg(rt.tx.clone());
    rt.mw_watcher = watcher::spawn_mw(rt.tx.clone());
    rt.jank_watcher = watcher::spawn_jank(rt.tx.clone());
    if rt.fg_watcher.is_none() {
        rt.log("foreground watcher: failed to spawn logcat");
    }
    if rt.mw_watcher.is_none() {
        rt.log("multi-window watcher: failed to spawn logcat");
    }
    if rt.jank_watcher.is_none() {
        rt.log("jank watcher: failed to spawn logcat");
    }
    // read-only telemetry (battery / thermal / GPU busy) for diag + guards
    env::spawn(msg_tx.clone());
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
            Ok(Msg::Env(snap)) => rt.on_env(snap),
            Ok(Msg::MaintDone(out)) => rt.on_maint_done(out),
            Ok(Msg::Jank { frames }) => rt.on_jank(frames),
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
            // profile-pack deploy (app upgrade): a changed profiles.json must
            // re-evaluate, or new catalog keys would wait for the next switch
            let pm = mtime(&rt.state_dir.join("profiles.json"));
            if pm != rt.profiles_mtime {
                rt.profiles_mtime = pm;
                if pm.is_some() {
                    rt.log("profiles reloaded (mtime)");
                    let fg = rt.last_real.clone().or_else(|| rt.last_fg.clone());
                    rt.evaluate("profiles", fg);
                }
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
    if let Some(w) = rt.jank_watcher.as_mut() {
        w.stop();
    }
    // releases bridge holds on a clean exit; an abrupt kill self-heals next
    // start via holds.json (recover + first evaluate decides RESTORE)
    rt.bridge.release_all();
    rt.publisher.emit(&Event::Bye);
    Ok(())
}
