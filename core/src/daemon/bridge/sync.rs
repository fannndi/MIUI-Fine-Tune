//! Sync context, pure gates and the MIUI mode sync IO.
//!
//! Inherent impl for [`Bridge`](super::Bridge) (defined in `mod.rs`); the
//! state machine lives in `holds.rs`. Every sync runs under the bridge
//! Mutex held by the caller.

use super::holds::{PerfAction, PowerMode, RefreshAction, SaverAction};
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
    /// Per-app refresh follow (`Settings.System user_refresh_rate`).
    pub sync_refresh: bool,
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
    /// Charge-to-100%-once: skip the limit until the next unplug.
    pub charge_once: bool,
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

    /// Per-app refresh follow (`Settings.System user_refresh_rate`): the
    /// capture fires on the first write, survives daemon restarts and is
    /// released on app exit / Service OFF / recovery.
    pub(super) fn sync_refresh(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = refresh_want(ctx);
        let live = settings::read("system", settings::REFRESH_KEY);
        let (next, action) = st.holds.request_refresh(live, want);
        let changed = next != st.holds;
        match action {
            RefreshAction::Write(v) => {
                let _ = settings::put("system", settings::REFRESH_KEY, &v);
                let hz: u32 = v.parse().unwrap_or(120);
                // instant panel switch first (SF dfps transaction), then the
                // MIUI helper syncs the HAL's own state (~1 s)
                if let Err(e) = settings::apply_panel_refresh(hz) {
                    self.log_event(format!("refresh panel switch failed: {e}"));
                }
                if let Err(e) = settings::apply_refresh_fps(hz) {
                    self.log_event(format!("refresh fps helper failed: {e}"));
                }
                let who = if !ctx.screen_on {
                    "sleep".to_string()
                } else {
                    ctx.last_real.clone().unwrap_or_else(|| "?".into())
                };
                self.log_event(format!("refresh follow {v} Hz ({who})"));
            }
            RefreshAction::Restore(saved) => match &saved {
                Some(v) => {
                    let _ = settings::put("system", settings::REFRESH_KEY, v);
                    if let Ok(hz) = v.parse::<u32>() {
                        if let Err(e) = settings::apply_panel_refresh(hz) {
                            self.log_event(format!("refresh panel switch failed: {e}"));
                        }
                        if let Err(e) = settings::apply_refresh_fps(hz) {
                            self.log_event(format!("refresh fps helper failed: {e}"));
                        }
                    }
                    self.log_event(format!("refresh restored ({v})"));
                }
                None => {
                    let _ = settings::delete("system", settings::REFRESH_KEY);
                    // no captured value: the panel default is 120 (mode id 1)
                    if let Err(e) = settings::apply_panel_refresh(120) {
                        self.log_event(format!("refresh panel switch failed: {e}"));
                    }
                    if let Err(e) = settings::apply_refresh_fps(120) {
                        self.log_event(format!("refresh fps helper failed: {e}"));
                    }
                    self.log_event("refresh restored (no user value, default 120)".into());
                }
            },
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

/// Fixed refresh while the panel is off: a low rate avoids a 60/120 burst
/// on the wake transition (the user asked for 30 Hz in the off-screen mode).
const REFRESH_SLEEP_HZ: u32 = 30;

/// Refresh follow: master ON, then
/// - screen off -> the fixed sleep value,
/// - visible & unlocked & Dynamic ON -> the app entry's value
///   ("Default"/absent releases; MIUI/system keeps control),
/// - anything else -> release (restore the captured user value).
fn refresh_want(ctx: &SyncCtx) -> Option<u32> {
    if !ctx.sync_refresh {
        return None;
    }
    if !ctx.screen_on {
        return Some(REFRESH_SLEEP_HZ);
    }
    if ctx.locked || !ctx.dynamic {
        return None;
    }
    fg_app_profile(ctx).and_then(|e| e.refresh_target())
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
            sync_refresh: true,
            game_checker: true,
            app_map: crate::daemon::test_util::map(map),
            app_profiles: Default::default(),
            bypass_floor_pct: 30,
            dnd_granted: false,
            battery_pct: None,
            charging: None,
            charge_limit: false,
            charge_limit_pct: 80,
            charge_once: false,
        }
    }

    fn ctx_profiles(profiles: &[(&str, AppProfile)], fg: Option<&str>) -> SyncCtx {
        let mut c = ctx(&[], fg);
        c.app_profiles = profiles
            .iter()
            .map(|(p, a)| (p.to_string(), a.clone()))
            .collect();
        c
    }

    #[test]
    fn refresh_gate_per_app_default_and_sleep() {
        // no entry -> release (Default / MIUI keeps control)
        assert_eq!(refresh_want(&ctx(&[], Some("com.x"))), None);
        // entry with a value -> that value
        let app60 = AppProfile {
            refresh_hz: Some(60),
            ..Default::default()
        };
        assert_eq!(
            refresh_want(&ctx_profiles(&[("com.p", app60.clone())], Some("com.p"))),
            Some(60)
        );
        // entry without a value (Default) -> release
        assert_eq!(
            refresh_want(&ctx_profiles(
                &[("com.d", AppProfile::default())],
                Some("com.d")
            )),
            None
        );
        // invalid value (e.g. 75) -> release (validation filter)
        let bad = AppProfile {
            refresh_hz: Some(75),
            ..Default::default()
        };
        assert_eq!(
            refresh_want(&ctx_profiles(&[("com.b", bad)], Some("com.b"))),
            None
        );
        // screen off -> sleep value (non-app rule, works with Dynamic OFF)
        let mut c = ctx_profiles(&[("com.p", app60.clone())], Some("com.p"));
        c.screen_on = false;
        assert_eq!(refresh_want(&c), Some(30));
        let mut c = ctx(&[], Some("com.x"));
        c.screen_on = false;
        c.dynamic = false;
        assert_eq!(refresh_want(&c), Some(30));
        // locked -> release
        let mut c = ctx_profiles(&[("com.p", app60.clone())], Some("com.p"));
        c.locked = true;
        assert_eq!(refresh_want(&c), None);
        // Dynamic OFF -> release (app entries are app-driven)
        let mut c = ctx_profiles(&[("com.p", app60.clone())], Some("com.p"));
        c.dynamic = false;
        assert_eq!(refresh_want(&c), None);
        // master off -> release even with a value
        let mut c = ctx_profiles(&[("com.p", app60)], Some("com.p"));
        c.sync_refresh = false;
        assert_eq!(refresh_want(&c), None);
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
