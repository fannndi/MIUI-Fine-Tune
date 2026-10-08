//! MIUI bridge: perf mirror + saver follow + game-mode checker.
//!
//! Port of the Kotlin `MiStateBridge` + `MiBridgeState` pair. The daemon
//! runs as root, so reads/writes go through the `settings` CLI.
//!
//! Harmony rules:
//! - Only two write targets exist: `Settings.Global low_power` (live effect)
//!   and the `Settings.System power_mode` mirror of MIUI's performance
//!   switch. The real property (`persist.sys.aries.power_profile`) is
//!   SELinux-locked — it is never attempted.
//! - Hold/restore: the first hold captures the user's live value as the
//!   restore point; the release writes it back. While a hold is active the
//!   arbiter sees the USER's value (`user_saver`), never our own write.
//! - Everything app-driven is gated by the Dynamic Profile switch
//!   (`ctx.dynamic`); Ultra saver retires the daemon entirely (restore).
//!
//! Threading: one `Bridge` shared by the main loop (user_saver read), the
//! bridge sync thread (sync) and the worker (release_all on restore). A
//! single Mutex serializes all state + IO — the old "two syncs raced and
//! yo-yoed the mode" bug class is structurally impossible.

use super::proto::{Event, Publisher};
use super::settings;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::{Arc, Mutex};

/// MIUI's own performance switch values (Settings.System mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerMode {
    #[serde(rename = "middle")]
    Balanced,
    #[serde(rename = "high")]
    Performance,
}

impl PowerMode {
    pub fn of(raw: &str) -> PowerMode {
        if raw == "high" {
            PowerMode::Performance
        } else {
            PowerMode::Balanced
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            PowerMode::Balanced => "middle",
            PowerMode::Performance => "high",
        }
    }
}

#[derive(Debug)]
pub enum PerfAction {
    None,
    Write,
    Keep,
    Restore,
}

#[derive(Debug)]
pub enum SaverAction {
    None,
    TurnOn,
    Keep,
    Restore,
}

/// Hold/restore state machine (pure, unit tested).
#[derive(Debug, Clone, PartialEq)]
struct Holds {
    perf_held: bool,
    perf_saved: PowerMode,
    saver_held: bool,
    saver_saved: bool,
}

impl Default for Holds {
    fn default() -> Self {
        Holds { perf_held: false, perf_saved: PowerMode::Balanced, saver_held: false, saver_saved: false }
    }
}

impl Holds {
    /// `live` = current switch state; `want` = bridge wants Performance.
    fn request_perf(&self, live: PowerMode, want: bool) -> (Holds, PerfAction) {
        match (want, self.perf_held) {
            (true, false) => (
                Holds { perf_held: true, perf_saved: live, ..self.clone() },
                if live == PowerMode::Performance { PerfAction::Keep } else { PerfAction::Write },
            ),
            (true, true) => (self.clone(), PerfAction::Keep),
            (false, true) => (Holds { perf_held: false, ..self.clone() }, PerfAction::Restore),
            (false, false) => (self.clone(), PerfAction::None),
        }
    }

    /// `live` = current battery-saver state; `want` = bridge wants ON.
    fn request_saver(&self, live: bool, want: bool) -> (Holds, SaverAction) {
        match (want, self.saver_held) {
            (true, false) => (
                Holds { saver_held: true, saver_saved: live, ..self.clone() },
                if live { SaverAction::Keep } else { SaverAction::TurnOn },
            ),
            (true, true) => (self.clone(), SaverAction::Keep),
            (false, true) => (Holds { saver_held: false, ..self.clone() }, SaverAction::Restore),
            (false, false) => (self.clone(), SaverAction::None),
        }
    }

    /// The saver value the arbiter should treat as USER intent: while we
    /// hold the flag for a mapped app, the captured value stands in.
    fn user_saver(&self, live: bool) -> bool {
        if self.saver_held {
            self.saver_saved
        } else {
            live
        }
    }
}

/// Persisted holds (crash-safe restore points), `holds.json` in the state dir.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HoldsFile {
    perf_held: bool,
    perf_saved: String,
    saver_held: bool,
    saver_saved: bool,
}

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
    pub app_map: std::collections::BTreeMap<String, String>,
}

/// Shared bridge (state + IO), one Mutex serializes everything.
pub struct Bridge {
    holds_path: PathBuf,
    publisher: Arc<Publisher>,
    state: Mutex<State>,
    /// Lock-free attribution mirror: -1 = no hold (arbiter sees live),
    /// 0/1 = the captured user value while a hold is active. Kept in sync
    /// with `state` so the decision loop never blocks on the bridge Mutex.
    saver_override: AtomicI8,
}

struct State {
    holds: Holds,
    /// Game-mode warning dedupe: one notification per game session.
    game_warned_for: Option<String>,
}

impl Bridge {
    /// Loads persisted holds from a previous run (crash recovery).
    pub fn recover(state_dir: &Path, publisher: Arc<Publisher>) -> Arc<Bridge> {
        let holds_path = state_dir.join("holds.json");
        let holds = fs::read_to_string(&holds_path)
            .ok()
            .and_then(|s| serde_json::from_str::<HoldsFile>(&s).ok())
            .map(|f| Holds {
                perf_held: f.perf_held,
                perf_saved: PowerMode::of(&f.perf_saved),
                saver_held: f.saver_held,
                saver_saved: f.saver_saved,
            })
            .unwrap_or_default();
        if holds.perf_held || holds.saver_held {
            publisher.log("bridge: recovered holds from previous run");
        }
        let bridge = Arc::new(Bridge {
            holds_path,
            publisher,
            state: Mutex::new(State { holds, game_warned_for: None }),
            saver_override: AtomicI8::new(-1),
        });
        bridge.refresh_attribution();
        bridge
    }

    /// User-intent saver value for the arbiter: lock-free read (the refresh
    /// happens after every holds mutation).
    pub fn user_saver(&self, live: bool) -> bool {
        match self.saver_override.load(Ordering::Relaxed) {
            0 => false,
            1 => true,
            _ => live,
        }
    }

    fn refresh_attribution(&self) {
        let st = self.lock();
        // tri-state mirror: -1 no hold; 0/1 = the captured user value.
        let v: i8 = if st.holds.saver_held {
            if st.holds.user_saver(false) {
                1
            } else {
                0
            }
        } else {
            -1
        };
        self.saver_override.store(v, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn log_event(&self, msg: String) {
        self.publisher.log(&format!("bridge: {msg}"));
        self.publisher.emit(&Event::Bridge { msg });
    }

    /// One full sync: perf mirror, saver follow, game-mode checker.
    /// Serialized by the internal Mutex; safe to call after every decision.
    pub fn sync(&self, ctx: &SyncCtx) {
        // The lock is held across the settings execs so two syncs can never
        // interleave their read/write pairs (the old yo-yo bug class).
        let (perf_changed, saver_changed) = {
            let mut st = self.lock();
            let perf_changed = self.sync_perf(&mut st, ctx);
            let saver_changed = self.sync_saver(&mut st, ctx);
            self.game_check(&mut st, ctx);
            (perf_changed, saver_changed)
        };
        if perf_changed || saver_changed {
            self.refresh_attribution();
            let st = self.lock();
            self.persist(&st);
        }
    }

    fn sync_perf(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = perf_want(ctx);
        let live = read_power_mode();
        let (next, action) = st.holds.request_perf(live, want);
        let changed = next != st.holds;
        match action {
            PerfAction::Write => {
                let _ = settings::put("system", settings::POWER_MODE_KEY, PowerMode::Performance.key());
                self.publisher.log("bridge: MIUI perf mirror ON (game)");
            }
            PerfAction::Restore => {
                let _ = settings::put("system", settings::POWER_MODE_KEY, next.perf_saved.key());
                self.publisher.log(&format!(
                    "bridge: MIUI perf mirror restored ({})",
                    next.perf_saved.key()
                ));
            }
            _ => {}
        }
        st.holds = next;
        changed
    }

    fn sync_saver(&self, st: &mut State, ctx: &SyncCtx) -> bool {
        let want = saver_want(ctx);
        let live = settings::read_bool("global", settings::SAVER_KEY).unwrap_or(false);
        let (next, action) = st.holds.request_saver(live, want);
        let changed = next != st.holds;
        match action {
            SaverAction::TurnOn => {
                let _ = settings::put("global", settings::SAVER_KEY, "1");
                self.publisher.log(&format!(
                    "bridge: MIUI saver ON ({})",
                    ctx.last_real.as_deref().unwrap_or("?")
                ));
            }
            SaverAction::Restore => {
                let _ = settings::put(
                    "global",
                    settings::SAVER_KEY,
                    if next.saver_saved { "1" } else { "0" },
                );
                self.publisher.log("bridge: MIUI saver restored");
            }
            _ => {}
        }
        st.holds = next;
        changed
    }

    /// Game-mode checker: a mapped game in front + a held thermal scenario
    /// means MIUI Game Booster is still fighting our profile -> warn once
    /// per session (the app shows the notification).
    fn game_check(&self, st: &mut State, ctx: &SyncCtx) {
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
            self.publisher.emit(&Event::GameModeConflict { pkg: pkg.clone() });
        }
    }

    /// Writes the user's captured values back (teardown/retire/restore path).
    pub fn release_all(&self) {
        let changed = {
            let mut st = self.lock();
            let mut changed = false;
            if st.holds.perf_held {
                let _ = settings::put("system", settings::POWER_MODE_KEY, st.holds.perf_saved.key());
                self.log_event(format!("MIUI perf mirror restored ({})", st.holds.perf_saved.key()));
                st.holds.perf_held = false;
                changed = true;
            }
            if st.holds.saver_held {
                let _ = settings::put(
                    "global",
                    settings::SAVER_KEY,
                    if st.holds.saver_saved { "1" } else { "0" },
                );
                self.log_event("MIUI saver restored".into());
                st.holds.saver_held = false;
                changed = true;
            }
            changed
        };
        if changed {
            self.refresh_attribution();
            let st = self.lock();
            self.persist(&st);
        }
    }

    /// Atomic write (tmp + rename): a crash mid-write must never corrupt
    /// the restore points.
    fn persist(&self, st: &State) {
        let file = HoldsFile {
            perf_held: st.holds.perf_held,
            perf_saved: st.holds.perf_saved.key().to_string(),
            saver_held: st.holds.saver_held,
            saver_saved: st.holds.saver_saved,
        };
        let Ok(s) = serde_json::to_string_pretty(&file) else { return };
        let tmp = self.holds_path.with_extension("json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &self.holds_path);
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
    ctx.last_real.as_deref().and_then(|p| ctx.app_map.get(p)).map(|s| s.as_str())
}

/// Perf mirror wants Performance: Dynamic ON + sync ON + mapped game in
/// front + screen visible & unlocked.
fn perf_want(ctx: &SyncCtx) -> bool {
    ctx.dynamic && ctx.sync_perf && mapped_profile(ctx) == Some("game") && ctx.screen_on && !ctx.locked
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

    // --- state machine (ported 1:1 from MiBridgeStateTest) -----------------

    #[test]
    fn perf_hold_captures_user_value_then_restores_it() {
        let s = Holds::default();
        // user was on Balanced, game enters -> WRITE
        let (n, a) = s.request_perf(PowerMode::Balanced, true);
        assert!(matches!(a, PerfAction::Write));
        assert!(n.perf_held);
        assert_eq!(n.perf_saved, PowerMode::Balanced);

        // while held, repeated requests keep (no flash spam)
        let (n2, a2) = n.request_perf(PowerMode::Performance, true);
        assert!(matches!(a2, PerfAction::Keep));
        assert_eq!(n2.perf_saved, PowerMode::Balanced);

        // game leaves -> RESTORE to the captured user value
        let (n3, a3) = n2.request_perf(PowerMode::Performance, false);
        assert!(matches!(a3, PerfAction::Restore));
        assert!(!n3.perf_held);
        assert_eq!(n3.perf_saved, PowerMode::Balanced);
    }

    #[test]
    fn perf_hold_when_user_already_on_performance_keeps_without_write() {
        let s = Holds::default();
        let (n, a) = s.request_perf(PowerMode::Performance, true);
        assert!(matches!(a, PerfAction::Keep));
        assert!(n.perf_held);
        assert_eq!(n.perf_saved, PowerMode::Performance);
    }

    #[test]
    fn saver_hold_turns_on_then_restores() {
        let s = Holds::default();
        let (n, a) = s.request_saver(false, true);
        assert!(matches!(a, SaverAction::TurnOn));
        assert!(n.saver_held);
        assert!(!n.saver_saved);

        let (n2, a2) = n.request_saver(true, false);
        assert!(matches!(a2, SaverAction::Restore));
        assert!(!n2.saver_held);
    }

    #[test]
    fn user_saver_attribution_our_hold_is_invisible_to_arbiter() {
        let s = Holds::default();
        let (n, _) = s.request_saver(false, true);
        // while held, the arbiter must NOT see our write as user intent
        assert!(!n.user_saver(true));
        // after release it sees the live flag again
        let (n2, _) = n.request_saver(true, false);
        assert!(n2.user_saver(true));
    }

    #[test]
    fn user_saver_own_choice_passes_through() {
        let s = Holds::default();
        assert!(s.user_saver(true));
        assert!(!s.user_saver(false));
    }

    #[test]
    fn recovered_hold_restores_captured_value_not_live() {
        // daemon died while holding (user's saver was off, ours is on)
        let s = Holds { saver_held: true, saver_saved: false, ..Default::default() };
        let (n, a) = s.request_saver(true, false);
        assert!(matches!(a, SaverAction::Restore));
        assert!(!n.saver_saved);
    }

    #[test]
    fn power_mode_roundtrip() {
        assert_eq!(PowerMode::of("high"), PowerMode::Performance);
        assert_eq!(PowerMode::of("middle"), PowerMode::Balanced);
        assert_eq!(PowerMode::of("anything-else"), PowerMode::Balanced);
        assert_eq!(PowerMode::Performance.key(), "high");
    }

    // --- pure gate rules (dynamic/profile/screen combinations) -------------

    fn ctx(map: &[(&str, &str)], fg: Option<&str>) -> SyncCtx {
        SyncCtx {
            last_real: fg.map(str::to_string),
            screen_on: true,
            locked: false,
            dynamic: true,
            sync_perf: true,
            sync_saver: true,
            game_checker: true,
            app_map: super::super::test_util::map(map),
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
    fn holds_file_roundtrip() {
        let dir = super::super::test_util::tmpdir("bridge-holds");
        // simulate: write holds then recover
        let f = HoldsFile {
            perf_held: true,
            perf_saved: "high".into(),
            saver_held: true,
            saver_saved: true,
        };
        std::fs::write(dir.join("holds.json"), serde_json::to_string(&f).unwrap()).unwrap();
        let publisher = Publisher::new();
        let bridge = Bridge::recover(&dir, publisher);
        let st = bridge.lock();
        assert!(st.holds.perf_held);
        assert_eq!(st.holds.perf_saved, PowerMode::Performance);
        assert!(st.holds.saver_held);
        assert!(st.holds.saver_saved);
        drop(st);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
