//! The decision point and the supervisor timer.
//!
//! `evaluate` is the single place that turns world state into an arbiter
//! decision; `supervise` runs every 3 s (periodic re-evaluate + watcher
//! health + fallback peek). Inherent impl for [`Runtime`](super::Runtime).

use super::arbiter::{self, ArbiterInput, Decision, SLEEP_PROFILE};
use super::bridge::SyncCtx;
use super::config::DaemonConfig;
use super::watcher;
use super::worker::{self, Work};
use super::{Runtime, PERIODIC_TICKS};
use crate::engine::env::EnvSnapshot;
use std::time::Instant;

/// Thermal-guard release hysteresis: once engaged at the ceiling, the guard
/// releases only after the CPU drops this many °C below it.
pub const THERMAL_HYSTERESIS_C: f32 = 5.0;

/// Battery guard predicate: enabled, percent known, at/below the floor and
/// not charging (unknown charging counts as discharging — conservative).
pub(super) fn battery_low(cfg: &DaemonConfig, env: &EnvSnapshot) -> bool {
    cfg.guard_battery
        && env
            .battery_pct
            .map(|p| p <= cfg.battery_floor_pct)
            .unwrap_or(false)
        && env.charging != Some(true)
}

impl Runtime {
    /// Thermal guard with hysteresis: engage at the ceiling, release below
    /// ceiling - [`THERMAL_HYSTERESIS_C`]. Unknown temperature keeps the
    /// previous state (a missing sensor must never flap the profile).
    pub(super) fn thermal_high(&mut self, enabled: bool, ceiling: f32) -> bool {
        if !enabled {
            self.thermal_stepped = false;
            return false;
        }
        let stepped = self.thermal_stepped;
        match self.env.cpu_temp_c {
            Some(t) if t >= ceiling => {
                if !stepped {
                    self.log(&format!(
                        "guard: CPU {t:.1} °C >= {ceiling:.0} °C — thermal step-down"
                    ));
                }
                self.thermal_stepped = true;
                true
            }
            Some(t) if stepped && t <= ceiling - THERMAL_HYSTERESIS_C => {
                self.log(&format!("guard: CPU {t:.1} °C cooled — thermal release"));
                self.thermal_stepped = false;
                false
            }
            _ => stepped,
        }
    }

    /// The one decision point. `fg_override` mirrors the old `evaluate`:
    /// config toggles and unlocks evaluate for the *real* package even
    /// though our own (transient) app is technically in front.
    pub(super) fn evaluate(&mut self, trigger: &str, fg_override: Option<String>) {
        if self.retired {
            return;
        }
        let cfg = self.config.get().clone();
        if !cfg.enabled {
            // the app owns lifecycle: it stops the service (and thus us)
            return;
        }
        let battery_low = battery_low(&cfg, &self.env);
        let thermal_high = self.thermal_high(cfg.guard_thermal, cfg.thermal_ceiling_c);
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
            battery_low,
            thermal_high,
        };
        match arbiter::decide(&input) {
            Decision::None => {
                self.log(&format!("evaluate({trigger}): no-op"));
                self.emit_decision(trigger, "none", None, None);
            }
            Decision::Apply { profile, reason } => {
                self.log(&format!("evaluate({trigger}): -> {profile} ({reason})"));
                self.emit_decision(
                    trigger,
                    "apply",
                    Some(profile.clone()),
                    Some(reason.clone()),
                );
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
                self.log(&format!(
                    "evaluate({trigger}): MIUI ultra saver — retiring service"
                ));
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

    /// 3 s supervisor: periodic re-evaluate + watcher health + fallback peek.
    pub(super) fn supervise(&mut self, ticks: u64) {
        if ticks.is_multiple_of(PERIODIC_TICKS) && self.screen_on && !self.locked {
            self.evaluate("periodic", None);
        }
        // stream health
        let fg_alive = self
            .fg_watcher
            .as_ref()
            .map(|w| w.is_alive())
            .unwrap_or(false);
        let mw_alive = self
            .mw_watcher
            .as_ref()
            .map(|w| w.is_alive())
            .unwrap_or(false);
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
