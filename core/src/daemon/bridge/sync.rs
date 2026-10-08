//! Sync context, pure gates and the MIUI mode sync IO.
//!
//! Inherent impl for [`Bridge`](super::Bridge) (defined in `mod.rs`); the
//! state machine lives in `holds.rs`. Every sync runs under the bridge
//! Mutex held by the caller.

use super::holds::{PerfAction, PowerMode, SaverAction};
use super::{Bridge, State};
use crate::daemon::config::AppProfile;
use crate::daemon::proto::Event;
use crate::daemon::settings;
use std::fs;

/// Everything the sync needs about the current world.
#[derive(Debug, Clone)]
pub struct SyncCtx {
    pub last_real: Option<String>,
    pub screen_on: bool,
    pub locked: bool,
    pub dynamic: bool,
    pub sync_perf: bool,
    pub sync_saver: bool,
    pub game_checker: bool,
    /// Merged app map (`app_profiles[*].profile` over `app_map`).
    pub app_map: std::collections::BTreeMap<String, String>,
    /// Apps Profile entries (software extras), keyed by package.
    pub app_profiles: std::collections::BTreeMap<String, AppProfile>,
    /// Bypass safety floor (clamped, 15..50).
    pub bypass_floor_pct: u8,
    /// The app granted Do Not Disturb access (set by the `dnd_access` cmd).
    pub dnd_granted: bool,
    /// Charge-guard inputs from the last env sample.
    pub battery_pct: Option<u8>,
    pub charging: Option<bool>,
    pub charge_limit: bool,
    pub charge_limit_pct: u8,
}

impl Bridge {
    pub(super) fn sync_perf(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = perf_want(ctx);
        let live = read_power_mode();
        let (next, action) = st.holds.request_perf(live, want);
        let changed = next != st.holds;
        match action {
            PerfAction::Write => {
                let _ = settings::put(
                    "system",
                    settings::POWER_MODE_KEY,
                    PowerMode::Performance.key(),
                );
                self.log_event("MIUI perf mirror ON (game)".into());
            }
            PerfAction::Restore => {
                let _ = settings::put("system", settings::POWER_MODE_KEY, next.perf_saved.key());
                self.log_event(format!(
                    "MIUI perf mirror restored ({})",
                    next.perf_saved.key()
                ));
            }
            _ => {}
        }
        st.holds = next;
        changed
    }

    pub(super) fn sync_saver(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = saver_want(ctx);
        let live = settings::read_bool("global", settings::SAVER_KEY).unwrap_or(false);
        let (next, action) = st.holds.request_saver(live, want);
        let changed = next != st.holds;
        match action {
            SaverAction::TurnOn => {
                let _ = settings::put("global", settings::SAVER_KEY, "1");
                self.log_event(format!(
                    "MIUI saver ON ({})",
                    ctx.last_real.as_deref().unwrap_or("?")
                ));
            }
            SaverAction::Restore => {
                let _ = settings::put(
                    "global",
                    settings::SAVER_KEY,
                    if next.saver_saved { "1" } else { "0" },
                );
                self.log_event("MIUI saver restored".into());
            }
            _ => {}
        }
        st.holds = next;
        changed
    }

    /// Game-mode checker: a mapped game in front + a held thermal scenario
    /// means MIUI Game Booster is still fighting our profile -> warn once
    /// per session (the app shows the notification).
    pub(super) fn game_check(&self, st: &mut State, ctx: &SyncCtx) {
        if !game_want(ctx) {
            st.game_warned_for = None;
            return;
        }
        if !read_game_mode_signature() {
            st.game_warned_for = None;
            return;
        }
        if st.game_warned_for == ctx.last_real {
            return;
        }
        st.game_warned_for = ctx.last_real.clone();
        if let Some(pkg) = &ctx.last_real {
            self.publisher
                .emit(&Event::GameModeConflict { pkg: pkg.clone() });
        }
    }
}

/// Root-free mirror read (Settings.System power_mode).
fn read_power_mode() -> PowerMode {
    settings::read("system", settings::POWER_MODE_KEY)
        .map(|v| PowerMode::of(&v))
        .unwrap_or(PowerMode::Balanced)
}

/// MIUI Game Booster signature: thermal scenario held (sconfig != 0).
fn read_game_mode_signature() -> bool {
    fs::read_to_string("/sys/class/thermal/thermal_message/sconfig")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .map(|v| v != 0)
        .unwrap_or(false)
}

// --- pure gates (unit tested; the sync functions above only wire IO) --------

/// The profile the app map assigns to the current foreground, if any.
fn mapped_profile(ctx: &SyncCtx) -> Option<&str> {
    ctx.last_real
        .as_deref()
        .and_then(|p| ctx.app_map.get(p))
        .map(|s| s.as_str())
}

/// The Apps Profile entry for the current foreground app, if any.
pub(super) fn fg_app_profile(ctx: &SyncCtx) -> Option<&AppProfile> {
    ctx.last_real
        .as_deref()
        .and_then(|p| ctx.app_profiles.get(p))
}

/// Perf mirror wants Performance: Dynamic ON + sync ON + mapped game in
/// front + screen visible & unlocked.
fn perf_want(ctx: &SyncCtx) -> bool {
    ctx.dynamic
        && ctx.sync_perf
        && mapped_profile(ctx) == Some("game")
        && ctx.screen_on
        && !ctx.locked
}

/// Saver follow wants the saver ON: Dynamic ON + sync ON + a powersave-mapped
/// app in front + screen visible & unlocked.
fn saver_want(ctx: &SyncCtx) -> bool {
    ctx.dynamic
        && ctx.sync_saver
        && mapped_profile(ctx) == Some("powersave")
        && ctx.screen_on
        && !ctx.locked
}

/// Game-mode checker is armed for a mapped game (Dynamic ON + checker ON).
fn game_want(ctx: &SyncCtx) -> bool {
    ctx.dynamic && ctx.game_checker && mapped_profile(ctx) == Some("game")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(map: &[(&str, &str)], fg: Option<&str>) -> SyncCtx {
        SyncCtx {
            last_real: fg.map(str::to_string),
            screen_on: true,
            locked: false,
            dynamic: true,
            sync_perf: true,
            sync_saver: true,
            game_checker: true,
            app_map: crate::daemon::test_util::map(map),
            app_profiles: Default::default(),
            bypass_floor_pct: 30,
            dnd_granted: false,
            battery_pct: None,
            charging: None,
            charge_limit: false,
            charge_limit_pct: 80,
        }
    }

    #[test]
    fn perf_gate_requires_mapped_game_and_visibility() {
        let c = ctx(&[("com.g", "game")], Some("com.g"));
        assert!(perf_want(&c));
        // unmapped app -> no
        assert!(!perf_want(&ctx(&[("com.g", "game")], Some("com.other"))));
        // powersave mapping -> no
        assert!(!perf_want(&ctx(&[("com.p", "powersave")], Some("com.p"))));
        // screen off / locked -> no
        let mut c2 = c.clone();
        c2.screen_on = false;
        assert!(!perf_want(&c2));
        let mut c3 = c.clone();
        c3.locked = true;
        assert!(!perf_want(&c3));
        // dynamic off -> no (v0.5 gate)
        let mut c4 = c.clone();
        c4.dynamic = false;
        assert!(!perf_want(&c4));
        // sync flag off -> no
        let mut c5 = c.clone();
        c5.sync_perf = false;
        assert!(!perf_want(&c5));
    }

    #[test]
    fn saver_gate_requires_mapped_powersave() {
        assert!(saver_want(&ctx(&[("com.p", "powersave")], Some("com.p"))));
        assert!(!saver_want(&ctx(&[("com.g", "game")], Some("com.g"))));
        assert!(!saver_want(&ctx(&[], Some("com.p"))));
        let mut c = ctx(&[("com.p", "powersave")], Some("com.p"));
        c.dynamic = false;
        assert!(!saver_want(&c), "dynamic OFF must stop the saver follow");
    }

    #[test]
    fn game_gate_requires_mapped_game_and_checker() {
        assert!(game_want(&ctx(&[("com.g", "game")], Some("com.g"))));
        assert!(!game_want(&ctx(&[("com.g", "game")], Some("com.other"))));
        let mut c = ctx(&[("com.g", "game")], Some("com.g"));
        c.game_checker = false;
        assert!(!game_want(&c));
        let mut c2 = ctx(&[("com.g", "game")], Some("com.g"));
        c2.dynamic = false;
        assert!(!game_want(&c2));
    }
}
