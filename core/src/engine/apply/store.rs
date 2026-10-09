//! State dir storage: snapshot/state JSON (atomic writes) + profile loading.

use super::{now_secs, read_json, write_json, SnapValue, Snapshot, State};
use crate::engine::catalog;
use crate::engine::plan::{OpStatus, PlannedOp};
use crate::engine::probe::{self, ProbeData};
use crate::engine::readback::normalize_snapshot;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Store {
            dir: dir.to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn snapshot_path(&self) -> PathBuf {
        self.dir.join("snapshot.json")
    }

    fn stock_path(&self) -> PathBuf {
        self.dir.join("stock.json")
    }

    /// Persistent union-stock map: one entry per catalog key the profiles
    /// ever touch; never consumed. Seeded once (first apply after install)
    /// and grown when a new profile version introduces a key. Reconcile
    /// ("auto-revive") writes these values for keys the active profile does
    /// not set; the CLI `restore` parks every key back to stock.
    pub fn load_stock(&self) -> Option<Snapshot> {
        read_json(&self.stock_path())
    }

    pub fn save_stock(&self, s: &Snapshot) -> Result<(), String> {
        write_json(&self.stock_path(), s)
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    pub fn load_snapshot(&self) -> Option<Snapshot> {
        read_json(&self.snapshot_path())
    }

    pub fn save_snapshot(&self, s: &Snapshot) -> Result<(), String> {
        write_json(&self.snapshot_path(), s)
    }

    pub fn load_state(&self) -> State {
        read_json(&self.state_path()).unwrap_or_default()
    }

    pub fn save_state(&self, s: &State) -> Result<(), String> {
        write_json(&self.state_path(), s)
    }

    /// Load user profiles from state dir, falling back to the embedded set.
    pub fn load_profiles(
        &self,
        override_path: Option<&Path>,
    ) -> Result<crate::engine::profile::ProfilesFile, String> {
        if let Some(p) = override_path {
            let s = fs::read_to_string(p).map_err(|e| format!("read {p:?}: {e}"))?;
            return crate::engine::profile::parse_profiles(&s);
        }
        let local = self.dir.join("profiles.json");
        if local.exists() {
            let s = fs::read_to_string(&local).map_err(|e| format!("read {local:?}: {e}"))?;
            return crate::engine::profile::parse_profiles(&s);
        }
        crate::engine::profile::parse_profiles(crate::DEFAULT_PROFILES_JSON)
    }
}

/// Snapshot originals for every key we are about to touch (first write wins:
/// the earliest value is the stock one and is never overwritten).
/// Seed/extend the persistent `stock.json` with every profile-union key.
/// Fallback chain when a key is missing: existing `stock.json` value ->
/// legacy `snapshot.json` value -> live device value (post-boot MIUI stock).
/// The device file for union keys that never appeared is only useful on a
/// clean boot (sysfs resets at boot); the installer's stock.json is seeded
/// right after the first `apply` on a v0.11 device.
pub fn ensure_stock(store: &Store, probe: &ProbeData) -> Result<(), String> {
    let profiles = store.load_profiles(None)?;
    let mut keys: Vec<&str> = profiles
        .profiles
        .iter()
        .flat_map(|p| p.params.keys().map(|k| k.as_str()))
        .collect();
    keys.sort_unstable();
    keys.dedup();

    // Snapshot baseline: prefer the existing stock.json, else migrate the
    // legacy session snapshot (pre-v0.11 devices).
    let snap = store.load_stock().or_else(|| store.load_snapshot());
    let created = snap.as_ref().map(|s| s.created).unwrap_or_else(now_secs);
    let device = snap
        .as_ref()
        .map(|s| s.device.clone())
        .unwrap_or_else(|| probe.device.device.clone());
    let mut snap = snap.unwrap_or_else(|| Snapshot {
        created,
        device,
        values: BTreeMap::new(),
    });
    let mut changed = false;
    for key in keys {
        // Refill a previously-empty capture (e.g. cfq tunables captured while
        // the deadline elevator was active — the node didn't exist then).
        if snap
            .values
            .get(key)
            .map(|v| !v.value.is_empty())
            .unwrap_or(false)
        {
            continue;
        }
        let Some(entry) = catalog::find(key) else {
            continue;
        };
        let value = normalize_snapshot(entry.kind, &probe::read(entry.path).unwrap_or_default());
        if value.is_empty() {
            // Node unavailable right now (elevator/governor-dependent): skip,
            // a later apply re-captures. Never store an empty "stock".
            continue;
        }
        snap.values.insert(
            key.to_owned(),
            SnapValue {
                path: entry.path.to_owned(),
                value,
            },
        );
        changed = true;
    }
    if changed {
        store.save_stock(&snap)?;
    }
    Ok(())
}

pub(super) fn ensure_snapshot(
    store: &Store,
    probe: &ProbeData,
    ops: &[PlannedOp],
) -> Result<(Snapshot, bool), String> {
    let existing = store.load_snapshot();
    let created_now = existing.is_none();
    let mut snap = existing.unwrap_or_else(|| Snapshot {
        created: now_secs(),
        device: probe.device.device.clone(),
        values: BTreeMap::new(),
    });
    let mut changed = false;
    for op in ops {
        if matches!(op.status, OpStatus::Locked(_)) {
            continue;
        }
        if snap.values.contains_key(&op.key) {
            continue;
        }
        let value = op
            .current
            .clone()
            .or_else(|| probe.entries.get(&op.key).and_then(|e| e.value.clone()))
            .unwrap_or_default();
        let kind = catalog::find(&op.key)
            .map(|e| e.kind)
            .unwrap_or(catalog::Kind::Text);
        let value = normalize_snapshot(kind, &value);
        snap.values.insert(
            op.key.clone(),
            SnapValue {
                path: op.path.clone(),
                value,
            },
        );
        changed = true;
    }
    if changed {
        store.save_snapshot(&snap)?;
    }
    Ok((snap, created_now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testutil::tmpdir;

    #[test]
    fn json_writes_are_atomic_and_valid() {
        // write_json must leave a valid file and never leave .tmp leftovers
        let dir = tmpdir("atomic");
        let store = Store::new(&dir);
        let st = State {
            active: Some("game".into()),
            updated: 7,
            last_mode: "apply".into(),
        };
        store.save_state(&st).unwrap();
        assert_eq!(store.load_state().active.as_deref(), Some("game"));
        // no stray tmp files in the state dir
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_roundtrip() {
        let dir = tmpdir("state");
        let store = Store::new(&dir);
        assert!(store.load_state().active.is_none());
        store
            .save_state(&State {
                active: Some("game".into()),
                updated: 42,
                last_mode: "apply".into(),
            })
            .unwrap();
        let st = store.load_state();
        assert_eq!(st.active.as_deref(), Some("game"));
        assert_eq!(st.updated, 42);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_profiles_load_without_file() {
        let dir = tmpdir("profiles");
        let store = Store::new(&dir);
        let p = store.load_profiles(None).unwrap();
        assert_eq!(p.profiles.len(), 5); // powersave, balance, game, sleep, boost
        let _ = fs::remove_dir_all(&dir);
    }
}
