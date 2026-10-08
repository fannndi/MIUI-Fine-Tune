//! Charge guard: pause charging at the configured limit (opt-in).
//!
//! The node (`battery_charging_enabled`) is the ROM's own user-facing charge
//! switch (init.target.rc chmod 0777 + chown system) and is cataloged as
//! Baseline; every write passes the engine guard. The hold captures the
//! previous value on the first pause and writes it back on release, so a
//! service-off/retire never leaves the phone unable to charge.
//!
//! Trigger: opt-in + charging at/above the limit; release at
//! limit - [`HYSTERESIS_PCT`] (the charger reports "not charging" while
//! paused, so the held state must not depend on the live status).

use super::holds::ChargeAction;
use super::{Bridge, State, SyncCtx};
use crate::engine::apply::guarded_write;
use crate::engine::env::default_root;
use std::fs;

/// Release threshold: resume charging this many % below the limit.
pub const HYSTERESIS_PCT: u8 = 5;

fn charge_node() -> String {
    default_root()
        .join("sys/class/power_supply/battery/battery_charging_enabled")
        .display()
        .to_string()
}

fn read_live() -> Option<String> {
    fs::read_to_string(charge_node())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Pure gate: start pausing when charging at/above the limit; keep pausing
/// (even though the status flips to "not charging") until the hysteresis
/// floor; anything else releases.
pub fn charge_pause_want(ctx: &SyncCtx, held: bool) -> bool {
    if !ctx.charge_limit {
        return false;
    }
    let pct = ctx.battery_pct.unwrap_or(0);
    if held {
        pct > ctx.charge_limit_pct.saturating_sub(HYSTERESIS_PCT)
    } else {
        ctx.charging == Some(true) && pct >= ctx.charge_limit_pct
    }
}

impl Bridge {
    /// One charge-guard sync; caller holds the bridge lock (like the others).
    pub(super) fn sync_charge(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = charge_pause_want(ctx, st.holds.charge_held);
        let live = read_live();
        let (next, action) = st.holds.request_charge(live, want);
        match action {
            ChargeAction::Pause => match guarded_write(&charge_node(), "0") {
                Ok(()) => {
                    self.log_event(format!(
                        "charge paused at {}% (limit {})",
                        ctx.battery_pct.unwrap_or(0),
                        ctx.charge_limit_pct
                    ));
                    st.holds = next;
                    true
                }
                Err(e) => {
                    self.log_event(format!("charge pause failed: {e}"));
                    false
                }
            },
            ChargeAction::Resume(saved) => {
                let value = saved.unwrap_or_else(|| "1".into());
                match guarded_write(&charge_node(), &value) {
                    Ok(()) => {
                        self.log_event(format!("charge resumed (node {value})"));
                        st.holds = next;
                        true
                    }
                    Err(e) => {
                        self.log_event(format!("charge resume failed: {e}"));
                        false
                    }
                }
            }
            ChargeAction::Keep => {
                let changed = next != st.holds;
                st.holds = next;
                changed
            }
            ChargeAction::None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(limit: bool, pct: u8, charging: Option<bool>, limit_pct: u8) -> SyncCtx {
        SyncCtx {
            last_real: None,
            screen_on: true,
            locked: false,
            dynamic: true,
            sync_perf: true,
            sync_saver: true,
            game_checker: true,
            app_map: Default::default(),
            battery_pct: Some(pct),
            charging,
            charge_limit: limit,
            charge_limit_pct: limit_pct,
        }
    }

    #[test]
    fn pause_starts_at_limit_while_charging() {
        assert!(charge_pause_want(&ctx(true, 80, Some(true), 80), false));
        assert!(charge_pause_want(&ctx(true, 95, Some(true), 80), false));
        assert!(!charge_pause_want(&ctx(true, 79, Some(true), 80), false));
        // not charging -> never start
        assert!(!charge_pause_want(&ctx(true, 90, Some(false), 80), false));
        assert!(!charge_pause_want(&ctx(true, 90, None, 80), false));
        // toggle off
        assert!(!charge_pause_want(&ctx(false, 95, Some(true), 80), false));
    }

    #[test]
    fn held_state_uses_hysteresis_and_ignores_status() {
        // while held, the charger reports "not charging" — keep pausing
        assert!(charge_pause_want(&ctx(true, 80, Some(false), 80), true));
        assert!(charge_pause_want(&ctx(true, 76, None, 80), true));
        // below the floor -> release
        assert!(!charge_pause_want(&ctx(true, 75, Some(false), 80), true));
        assert!(!charge_pause_want(&ctx(true, 60, None, 80), true));
        // toggle off while held -> release
        assert!(!charge_pause_want(&ctx(false, 90, Some(true), 80), true));
    }
}
