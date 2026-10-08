//! Sync context, pure gates and the MIUI mode sync IO.
//!
//! Inherent impl for [`Bridge`](super::Bridge) (defined in `mod.rs`); the
//! state machine lives in `holds.rs`. Every sync runs under the bridge
//! Mutex held by the caller.

use super::holds::{PerfAction, PowerMode, RefreshAction, SaverAction};
use super::{Bridge, State};
use crate::daemon::proto::Event;
use crate::daemon::settings;
use std::fs;

/// Refresh-rate targets for the follow feature.
pub const REFRESH_GAME_HZ: u32 = 120;
pub const REFRESH_SAVER_HZ: u32 = 60;

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
    pub sync_refresh: bool,
    pub app_map: std::collections::BTreeMap<String, String>,
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

    /// Refresh-rate follow: game -> 120 Hz, powersave-mapped app -> 60 Hz,
    /// anything else -> restore the user's own value. The hold captures the
    /// user's value on the first write and survives daemon restarts.
    pub(super) fn sync_refresh(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = refresh_want(ctx);
        let live = settings::read("system", settings::REFRESH_KEY);
        let (next, action) = st.holds.request_refresh(live, want);
        let changed = next != st.holds;
        match action {
            RefreshAction::Write(v) => {
                let _ = settings::put("system", settings::REFRESH_KEY, &v);
                self.log_event(format!(
                    "refresh follow {v} Hz ({})",
                    mapped_profile(ctx).unwrap_or("?")
                ));
            }
            RefreshAction::Restore(saved) => match &saved {
                Some(v) => {
                    let _ = settings::put("system", settings::REFRESH_KEY, v);
                    self.log_event(format!("refresh restored ({v})"));
                }
                None => {
                    let _ = settings::delete("system", settings::REFRESH_KEY);
                    self.log_event("refresh restored (user setting was unset)".into());
                }
            },
            _ => {}
        }
        st.holds = next;
        changed
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

/// Refresh follow: Dynamic ON + sync ON + visible & unlocked. Game -> 120,
/// powersave-mapped -> 60; every other app releases (restores the user value).
fn refresh_want(ctx: &SyncCtx) -> Option<u32> {
    if !(ctx.dynamic && ctx.sync_refresh && ctx.screen_on && !ctx.locked) {
        return None;
    }
    match mapped_profile(ctx) {
        Some("game") => Some(REFRESH_GAME_HZ),
        Some("powersave") => Some(REFRESH_SAVER_HZ),
        _ => None,
    }
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
            sync_refresh: true,
            app_map: crate::daemon::test_util::map(map),
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

    #[test]
    fn refresh_follow_maps_game_and_powersave() {
        assert_eq!(
            refresh_want(&ctx(&[("com.g", "game")], Some("com.g"))),
            Some(REFRESH_GAME_HZ)
        );
        assert_eq!(
            refresh_want(&ctx(&[("com.p", "powersave")], Some("com.p"))),
            Some(REFRESH_SAVER_HZ)
        );
        assert_eq!(
            refresh_want(&ctx(&[("com.b", "balance")], Some("com.b"))),
            None,
            "other profiles release the hold (user value returns)"
        );
        assert_eq!(
            refresh_want(&ctx(&[("com.g", "game")], Some("com.other"))),
            None
        );

        // gates mirror the perf mirror: dynamic, toggle, visibility
        let mut c = ctx(&[("com.g", "game")], Some("com.g"));
        c.sync_refresh = false;
        assert_eq!(refresh_want(&c), None);
        let mut c2 = ctx(&[("com.g", "game")], Some("com.g"));
        c2.dynamic = false;
        assert_eq!(refresh_want(&c2), None);
        let mut c3 = ctx(&[("com.p", "powersave")], Some("com.p"));
        c3.screen_on = false;
        assert_eq!(refresh_want(&c3), None);
        let mut c4 = ctx(&[("com.p", "powersave")], Some("com.p"));
        c4.locked = true;
        assert_eq!(refresh_want(&c4), None);
    }
}
