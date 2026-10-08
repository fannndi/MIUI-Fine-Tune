//! MIUI bridge: perf mirror + saver follow + game-mode checker.
//!
//! Files:
//! - `holds.rs`  PowerMode + hold/restore state machine + persisted file
//! - `sync.rs`   SyncCtx, pure gates, the sync IO (settings CLI)
//! - `mod.rs`    Bridge (shared state + one Mutex) + recover/release/persist
//!
//! Harmony rules:
//! - Only two write targets exist: `Settings.Global low_power` (live effect)
//!   and the `Settings.System power_mode` mirror of MIUI's performance
//!   switch. The real property (`persist.sys.aries.power_profile`) is
//!   SELinux-locked — it is never attempted.
//! - Everything app-driven is gated by the Dynamic Profile switch
//!   (`ctx.dynamic`); Ultra saver retires the daemon entirely (restore).
//!
//! Threading: one `Bridge` shared by the main loop (user_saver read), the
//! bridge sync thread (sync) and the worker (release_all on restore). A
//! single Mutex serializes all state + IO — the old "two syncs raced and
//! yo-yoed the mode" bug class is structurally impossible.

mod charge;
mod holds;
mod sync;

pub use holds::HoldsInfo;
pub use holds::PowerMode;
pub use sync::SyncCtx;

use super::proto::{Event, Publisher};
use crate::daemon::settings;
use holds::{Holds, HoldsFile};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::{Arc, Mutex};

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
        // Legacy F9 removal: a daemon killed while holding the user's
        // refresh value must give it back once, then the field disappears.
        migrate_legacy_refresh(&holds_path, &publisher);
        let holds = fs::read_to_string(&holds_path)
            .ok()
            .and_then(|s| serde_json::from_str::<HoldsFile>(&s).ok())
            .map(|f| Holds {
                perf_held: f.perf_held,
                perf_saved: PowerMode::of(&f.perf_saved),
                saver_held: f.saver_held,
                saver_saved: f.saver_saved,
                charge_held: f.charge_held,
                charge_saved: f.charge_saved,
            })
            .unwrap_or_default();
        if holds.perf_held || holds.saver_held || holds.charge_held {
            publisher.log("bridge: recovered holds from previous run");
        }
        let bridge = Arc::new(Bridge {
            holds_path,
            publisher,
            state: Mutex::new(State {
                holds,
                game_warned_for: None,
            }),
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

    /// Read-only hold state for `diag` (brief lock; never any IO).
    pub fn holds_info(&self) -> HoldsInfo {
        let st = self.lock();
        HoldsInfo {
            perf_held: st.holds.perf_held,
            perf_saved: st.holds.perf_saved.key().to_string(),
            saver_held: st.holds.saver_held,
            saver_saved: st.holds.saver_saved,
            charge_held: st.holds.charge_held,
            charge_saved: st.holds.charge_saved.clone(),
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
        let (perf_changed, saver_changed, charge_changed) = {
            let mut st = self.lock();
            let perf_changed = self.sync_perf(&mut st, ctx);
            let saver_changed = self.sync_saver(&mut st, ctx);
            let charge_changed = self.sync_charge(&mut st, ctx);
            self.game_check(&mut st, ctx);
            (perf_changed, saver_changed, charge_changed)
        };
        if perf_changed || saver_changed || charge_changed {
            self.refresh_attribution();
            let st = self.lock();
            self.persist(&st);
        }
    }
    /// Writes the user's captured values back (teardown/retire/restore path).
    pub fn release_all(&self) {
        let changed = {
            let mut st = self.lock();
            let mut changed = false;
            if st.holds.perf_held {
                let _ = settings::put(
                    "system",
                    settings::POWER_MODE_KEY,
                    st.holds.perf_saved.key(),
                );
                self.log_event(format!(
                    "MIUI perf mirror restored ({})",
                    st.holds.perf_saved.key()
                ));
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
            if st.holds.charge_held {
                let value = st.holds.charge_saved.clone().unwrap_or_else(|| "1".into());
                let node = crate::engine::env::default_root()
                    .join("sys/class/power_supply/battery/battery_charging_enabled");
                let _ = crate::engine::apply::guarded_write(&node.display().to_string(), &value);
                self.log_event(format!("charge resumed (node {value})"));
                st.holds.charge_held = false;
                st.holds.charge_saved = None;
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
            charge_held: st.holds.charge_held,
            charge_saved: st.holds.charge_saved.clone(),
        };
        let Ok(s) = serde_json::to_string_pretty(&file) else {
            return;
        };
        let tmp = self.holds_path.with_extension("json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &self.holds_path);
        }
    }
}

/// One-time migration for the removed F9 refresh-follow: a `holds.json`
/// written by an older daemon may still hold the user's refresh value.
/// Give it back once; the next persist drops the fields.
fn migrate_legacy_refresh(holds_path: &Path, publisher: &Publisher) {
    let Ok(raw) = fs::read_to_string(holds_path) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let held = v
        .get("refresh_held")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    if !held {
        return;
    }
    match v.get("refresh_saved").and_then(|s| s.as_str()) {
        Some(saved) if !saved.is_empty() => {
            let _ = settings::put("system", settings::REFRESH_KEY, saved);
            publisher.log(&format!("bridge: legacy F9 refresh restored ({saved})"));
        }
        _ => {
            let _ = settings::delete("system", settings::REFRESH_KEY);
            publisher.log("bridge: legacy F9 refresh restored (key was unset)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_file_roundtrip() {
        let dir = super::super::test_util::tmpdir("bridge-holds");
        // simulate: write holds then recover
        let f = HoldsFile {
            perf_held: true,
            perf_saved: "high".into(),
            saver_held: true,
            saver_saved: true,
            charge_held: true,
            charge_saved: Some("1".into()),
        };
        std::fs::write(dir.join("holds.json"), serde_json::to_string(&f).unwrap()).unwrap();
        let publisher = Publisher::new();
        let bridge = Bridge::recover(&dir, publisher);
        let st = bridge.lock();
        assert!(st.holds.perf_held);
        assert_eq!(st.holds.perf_saved, PowerMode::Performance);
        assert!(st.holds.saver_held);
        assert!(st.holds.saver_saved);
        assert!(st.holds.charge_held);
        assert_eq!(st.holds.charge_saved.as_deref(), Some("1"));
        drop(st);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
