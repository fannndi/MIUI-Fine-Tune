//! State dir storage: snapshot/state JSON (atomic writes) + profile loading.

use super::{now_secs, read_json, write_json, SnapValue, Snapshot, State};
use crate::engine::catalog;
use crate::engine::plan::{OpStatus, PlannedOp};
use crate::engine::probe::ProbeData;
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
        assert_eq!(p.profiles.len(), 4); // powersave, balance, game, sleep
        let _ = fs::remove_dir_all(&dir);
    }
}
