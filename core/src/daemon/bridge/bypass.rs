//! Per-app bypass charging: suspend charger input while the app is in front.
//!
//! `input_suspend=1` votes 0 mA on the USB input (USER_VOTER) — the device
//! runs on battery, so charging heat disappears but the battery drains; the
//! guard therefore releases at the configured floor (5 % hysteresis band)
//! and whenever the app leaves the foreground, the service stops or the
//! daemon recovers. Every write passes the engine's catalog guard
//! (`ALLOWED_EXACT`, Baseline).

use super::holds::BypassAction;
use super::sync::fg_app_profile;
use super::{Bridge, State, SyncCtx};
use crate::engine::apply::guarded_write;
use crate::engine::env::default_root;
use std::fs;

/// Release at/below the floor, re-engage this many % above it.
pub const HYSTERESIS_PCT: u8 = 5;

fn node() -> String {
    default_root()
        .join("sys/class/power_supply/battery/input_suspend")
        .display()
        .to_string()
}

fn read_live() -> Option<String> {
    fs::read_to_string(node())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn usb_online() -> bool {
    fs::read_to_string(default_root().join("sys/class/power_supply/usb/online"))
        .ok()
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn capacity() -> u8 {
    fs::read_to_string(default_root().join("sys/class/power_supply/battery/capacity"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Pure gate (unit tested): Dynamic ON + visible & unlocked + the per-app
/// toggle. Engaging needs the charger online and the battery above the
/// floor + hysteresis; while held, only the floor (with hysteresis) matters
/// — the charger may report "not charging" because of our own suspend.
pub fn bypass_want_pure(ctx: &SyncCtx, held: bool, pct: u8, usb_online: bool) -> bool {
    if !(ctx.dynamic && ctx.screen_on && !ctx.locked) {
        return false;
    }
    let Some(ap) = fg_app_profile(ctx) else {
        return false;
    };
    if !ap.bypass_charge {
        return false;
    }
    if held {
        pct > ctx.bypass_floor_pct
    } else {
        usb_online && pct >= ctx.bypass_floor_pct.saturating_add(HYSTERESIS_PCT)
    }
}

impl Bridge {
    /// One bypass sync; caller holds the bridge lock (like the others).
    pub(super) fn sync_bypass(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = bypass_want_pure(ctx, st.holds.bypass_held, capacity(), usb_online());
        let live = read_live();
        let (next, action) = st.holds.request_bypass(live, want);
        match action {
            BypassAction::Engage => match guarded_write(&node(), "1") {
                Ok(()) => {
                    self.log_event(format!(
                        "bypass charging ON ({}, {}%)",
                        ctx.last_real.as_deref().unwrap_or("?"),
                        capacity()
                    ));
                    st.holds = next;
                    true
                }
                Err(e) => {
                    self.log_event(format!("bypass charging failed: {e}"));
                    false
                }
            },
            BypassAction::Release(saved) => {
                let value = saved.unwrap_or_else(|| "0".into());
                match guarded_write(&node(), &value) {
                    Ok(()) => {
                        self.log_event(format!("bypass charging OFF (node {value})"));
                        st.holds = next;
                        true
                    }
                    Err(e) => {
                        self.log_event(format!("bypass release failed: {e}"));
                        false
                    }
                }
            }
            BypassAction::Keep => {
                let changed = next != st.holds;
                st.holds = next;
                changed
            }
            BypassAction::None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::AppProfile;

    fn ctx(apps: &[(&str, AppProfile)], fg: Option<&str>, floor: u8) -> SyncCtx {
        SyncCtx {
            last_real: fg.map(str::to_string),
            screen_on: true,
            locked: false,
            dynamic: true,
            sync_perf: true,
            sync_saver: true,
            game_checker: true,
            app_map: Default::default(),
            app_profiles: apps
                .iter()
                .map(|(p, a)| (p.to_string(), a.clone()))
                .collect(),
            bypass_floor_pct: floor,
            dnd_granted: false,
            battery_pct: None,
            charging: None,
            charge_limit: false,
            charge_limit_pct: 80,
            charge_once: false,
        }
    }

    fn bypass_app() -> AppProfile {
        AppProfile {
            bypass_charge: true,
            ..Default::default()
        }
    }

    #[test]
    fn engages_only_when_charging_above_the_floor_band() {
        let c = ctx(&[("com.g", bypass_app())], Some("com.g"), 30);
        // charging at 35+ -> engage
        assert!(bypass_want_pure(&c, false, 35, true));
        assert!(bypass_want_pure(&c, false, 80, true));
        // inside the hysteresis band -> not yet
        assert!(!bypass_want_pure(&c, false, 31, true));
        // no charger -> never engage
        assert!(!bypass_want_pure(&c, false, 80, false));
    }

    #[test]
    fn held_state_ignores_the_charger_and_releases_at_the_floor() {
        let c = ctx(&[("com.g", bypass_app())], Some("com.g"), 30);
        // while held the charger may report "not charging" -> keep
        assert!(bypass_want_pure(&c, true, 31, false));
        // at/below the floor -> release
        assert!(!bypass_want_pure(&c, true, 30, false));
        assert!(!bypass_want_pure(&c, true, 10, true));
    }

    #[test]
    fn gate_requires_foreground_toggle_and_visibility() {
        let c = ctx(&[("com.g", bypass_app())], Some("com.g"), 30);
        assert!(bypass_want_pure(&c, false, 50, true));
        // another app in front -> release
        let other = ctx(&[("com.g", bypass_app())], Some("com.other"), 30);
        assert!(!bypass_want_pure(&other, false, 50, true));
        // no app profile / toggle off
        let off = ctx(&[("com.g", AppProfile::default())], Some("com.g"), 30);
        assert!(!bypass_want_pure(&off, false, 50, true));
        // dynamic off / screen off / locked
        let mut c2 = c.clone();
        c2.dynamic = false;
        assert!(!bypass_want_pure(&c2, false, 50, true));
        let mut c3 = c.clone();
        c3.screen_on = false;
        assert!(!bypass_want_pure(&c3, false, 50, true));
        let mut c4 = c.clone();
        c4.locked = true;
        assert!(!bypass_want_pure(&c4, false, 50, true));
    }
}
