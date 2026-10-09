//! Command dispatch: IPC commands and watcher/peek messages.
//!
//! Inherent impl for [`Runtime`](super::Runtime); state lives in `mod.rs`,
//! the decision point in `evaluate.rs`.

use super::arbiter;
use super::proto::{DiagInfo, Watchers};
use super::watcher;
use super::worker::{self, Work};
use super::{Command, Event, Msg, Runtime, SLEEP_DELAY_MS, VERSION};
use std::time::{Duration, Instant};

impl Runtime {
    pub(super) fn on_command(&mut self, cmd: Command) -> bool {
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
                // explicit hint: force the reload (mtime granularity must
                // never swallow a write)
                if self.config.reload() {
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
                    self.screen_off_since = Some(Instant::now());
                    self.sleep_deadline =
                        Some(Instant::now() + Duration::from_millis(SLEEP_DELAY_MS));
                } else {
                    self.screen_off_since = None;
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
            Command::DndAccess { granted } => {
                self.dnd_granted = granted;
                self.log(&format!("DND access: {granted}"));
                // re-evaluate the DND bridge immediately (no profile decision)
                let _ = self.sync_tx.send(self.bridge_ctx());
            }
            Command::Ultra { on } => {
                self.ultra = on;
                self.log(&format!("MIUI extreme saver: {on}"));
                if on {
                    self.evaluate("ultra", None);
                }
            }
            Command::SetBase { profile } => {
                // the app writes config.json first; force-sync, then apply
                self.config.reload();
                self.log(&format!("set_base: {profile}"));
                let job = worker::Job {
                    profile,
                    reason: "base".into(),
                    src_pkg: None,
                    used_saver: false,
                    queued: Instant::now(),
                    // explicit user tap: re-plan even when already active
                    force: true,
                    watchdog: false,
                };
                let _ = self.work_tx.send(Work::Apply(job));
            }
            Command::Boost => {
                self.trigger_boost();
            }
            Command::Restore => {
                self.log("ipc: restore");
                let _ = self.work_tx.send(Work::Restore { retire: false });
            }
            Command::Diag => {
                let diag = DiagInfo {
                    version: VERSION,
                    pid: std::process::id(),
                    uptime_s: self.started.elapsed().as_secs(),
                    state_dir: self.state_dir.display().to_string(),
                    config_path: self.config_path.display().to_string(),
                    config: self.config.get().clone(),
                    env: self.env.clone(),
                    screen_on: self.screen_on,
                    locked: self.locked,
                    multi_window: self.multi_window,
                    foreground: self.last_fg.clone(),
                    active: self.active.clone(),
                    reason: self.reason.clone(),
                    watchers: Watchers {
                        fg: self
                            .fg_watcher
                            .as_ref()
                            .map(|w| w.is_alive())
                            .unwrap_or(false),
                        mw: self
                            .mw_watcher
                            .as_ref()
                            .map(|w| w.is_alive())
                            .unwrap_or(false),
                    },
                    holds: self.bridge.holds_info(),
                    stats_len: self.stats.entries.len(),
                };
                self.publisher.emit(&Event::Diag {
                    diag: Box::new(diag),
                });
            }
            Command::Stats => {
                self.publisher.emit(&Event::Stats {
                    entries: self.stats.entries.clone(),
                    heals_total: Some(self.stats.heals_total),
                    heals_last_t: Some(self.stats.heals_last_t),
                    heals_last_keys: Some(self.stats.heals_last_keys),
                });
            }
        }
        true
    }

    /// Foreground package event (own watcher or the app's forwarder).
    pub(super) fn on_fg(&mut self, pkg: String) {
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
    pub(super) fn on_seed(&mut self, peeked: Option<String>, trigger: &str) {
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
    pub(super) fn on_mw(&mut self, active: bool, other: Option<String>) {
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
    pub(super) fn peek_now(&mut self, trigger: &'static str) {
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("peek".into())
            .spawn(move || {
                let pkg = watcher::peek_fg();
                let _ = tx.send(Msg::Peek { pkg, trigger });
            });
    }
}
