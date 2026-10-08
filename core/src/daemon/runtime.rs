//! The event side of the main loop: state emission + apply/restore events.
//!
//! Inherent impl for [`Runtime`](super::Runtime) (defined in `mod.rs`).
//! The impl blocks are split for file size: this one holds logging +
//! snapshot emission + applied/restored handling; see `commands.rs`
//! (dispatch) and `evaluate.rs` (decision + supervisor).

use super::worker;
use super::worker::RestoredEvent;
use super::Runtime;
use super::{Event, Snapshot};
use crate::engine::env::EnvSnapshot;

impl Runtime {
    pub(super) fn log(&self, msg: &str) {
        self.publisher.log(msg);
    }

    pub(super) fn emit_state(&mut self, force: bool) {
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

    pub(super) fn emit_decision(
        &self,
        trigger: &str,
        action: &str,
        profile: Option<String>,
        reason: Option<String>,
    ) {
        self.publisher.emit(&Event::Decision {
            trigger: trigger.into(),
            action: action.into(),
            profile,
            reason,
        });
    }

    /// Fresh environment sample: keep it for guards + diag; forward to the
    /// app only when something changed (twice-a-minute silence otherwise).
    /// A guard-verdict flip (battery/thermal) re-evaluates immediately.
    pub(super) fn on_env(&mut self, snap: EnvSnapshot) {
        let changed = snap != self.env;
        let prev = (self.last_battery_low, self.thermal_stepped);
        self.env = snap;
        if changed {
            self.publisher.emit(&Event::Env {
                env: self.env.clone(),
            });
        }
        let cfg = self.config.get().clone();
        let bat = super::evaluate::battery_low(&cfg, &self.env);
        let therm = self.thermal_high(cfg.guard_thermal, cfg.thermal_ceiling_c);
        self.last_battery_low = bat;
        if (bat, therm) != prev && cfg.enabled && !self.retired {
            self.evaluate("env", None);
        }
    }

    pub(super) fn on_applied(&mut self, ev: worker::AppliedEvent) {
        if ev.ok {
            if ev.wrote == 0 {
                self.log(&format!("apply {}: in place", ev.profile));
            } else {
                self.log(&format!(
                    "apply {}: done in {}ms (settle {}ms)",
                    ev.profile, ev.ms, ev.settle_ms
                ));
            }
            // a real switch (not an "in place" confirmation) is history
            if self.active.as_deref() != Some(ev.profile.as_str()) {
                self.stats
                    .record(self.active.as_deref(), &ev.profile, &ev.reason, &self.env);
            }
            self.active = Some(ev.profile.clone());
            self.reason = Some(ev.reason.clone());
            self.src_pkg = ev.src_pkg.clone();
        } else {
            self.log(&format!(
                "apply {} failed after retry ({})",
                ev.profile, ev.failed
            ));
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

    pub(super) fn on_restored(&mut self, ev: RestoredEvent) {
        if ev.ok {
            if let Some(prev) = self.active.clone() {
                self.stats.record(
                    Some(&prev),
                    "stock",
                    if ev.retire { "retire" } else { "restore" },
                    &self.env,
                );
            }
            self.active = None;
            self.reason = None;
            self.src_pkg = None;
        }
        if ev.retire {
            self.log(&format!(
                "retire: restore ok={} failed={}",
                ev.ok, ev.failed
            ));
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
