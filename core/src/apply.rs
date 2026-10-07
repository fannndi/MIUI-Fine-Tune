//! Apply / restore / verify — the only code allowed to write, always guarded.
//!
//! Responsibility: snapshot originals before the first write of a key, apply
//! a validated [`Plan`] with read-back verification, restore stock, report
//! drift. Every write passes `catalog::guard_path`.
//! Non-goals: deciding values (profile.rs), UI.

use crate::catalog;
use crate::probe::{self, ProbeData};
use crate::profile::{readback_matches, Plan, PlannedOp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn default_state_dir() -> PathBuf {
    std::env::var("MIFINETUNE_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/data/adb/mifinetune"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapValue {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub created: u64,
    pub device: String,
    pub values: BTreeMap<String, SnapValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub active: Option<String>,
    pub updated: u64,
    #[serde(default)]
    pub last_mode: String,
}

impl Default for State {
    fn default() -> Self {
        State { active: None, updated: 0, last_mode: String::new() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub key: String,
    pub path: String,
    pub resolved: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub written: bool,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockedKey {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub mode: String, // "apply" | "restore"
    pub profile: Option<String>,
    pub wrote: usize,
    pub unchanged: usize,
    pub verified: usize,
    pub failed: usize,
    pub locked: Vec<LockedKey>,
    pub results: Vec<WriteResult>,
    pub ok: bool,
    pub snapshot_created: bool,
    pub active: Option<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Atomic JSON write: tmp file in the same directory + fsync + rename, so a
/// crash mid-write can never truncate snapshot.json/state.json (a lost
/// snapshot means a lost restore path).
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    let s = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("create {tmp:?}: {e}"))?;
        f.write_all(s.as_bytes()).map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync {tmp:?}: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Store { dir: dir.to_path_buf() }
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
    pub fn load_profiles(&self, override_path: Option<&Path>) -> Result<crate::profile::ProfilesFile, String> {
        if let Some(p) = override_path {
            let s = fs::read_to_string(p).map_err(|e| format!("read {p:?}: {e}"))?;
            return crate::profile::parse_profiles(&s);
        }
        let local = self.dir.join("profiles.json");
        if local.exists() {
            let s = fs::read_to_string(&local).map_err(|e| format!("read {local:?}: {e}"))?;
            return crate::profile::parse_profiles(&s);
        }
        crate::profile::parse_profiles(crate::DEFAULT_PROFILES_JSON)
    }
}

/// Snapshot originals for every key we are about to touch (first write wins:
/// the earliest value is the stock one and is never overwritten).
fn ensure_snapshot(
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
        if matches!(op.status, crate::profile::OpStatus::Locked(_)) {
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
        let kind = catalog::find(&op.key).map(|e| e.kind).unwrap_or(catalog::Kind::Text);
        let value = crate::profile::normalize_snapshot(kind, &value);
        snap.values.insert(op.key.clone(), SnapValue { path: op.path.clone(), value });
        changed = true;
    }
    if changed {
        store.save_snapshot(&snap)?;
    }
    Ok((snap, created_now))
}

fn write_one(op_path: &str, resolved: &str) -> Result<(), String> {
    catalog::guard_path(op_path)?;
    probe::write(op_path, resolved)
}

/// Pairs where the kernel enforces `second > first` on every write, and the
/// default (alphabetical) order writes `first` before `second`.
const KERNEL_SAFE_PAIRS: [(&str, &str); 2] = [
    ("kernel.sched_downmigrate", "kernel.sched_upmigrate"),
    ("gpu.max_pwrlevel", "gpu.min_pwrlevel"),
];

/// Reorder items so every kernel-validated pair is written safely:
/// if `want_second > cur_first` write `second` first (then `second > first`
/// holds); otherwise `first` first (then `first < cur_second` holds, since
/// `want_first < want_second <= cur_first < cur_second`).
fn ordered_pairs_with<T, K, W>(
    mut items: Vec<T>,
    key_of: K,
    want_of: W,
    current: impl Fn(&str) -> Option<i64>,
) -> Vec<T>
where
    K: Fn(&T) -> &str,
    W: Fn(&T) -> Option<i64>,
{
    for (first_key, second_key) in KERNEL_SAFE_PAIRS {
        let i_first = items.iter().position(|t| key_of(t) == first_key);
        let i_second = items.iter().position(|t| key_of(t) == second_key);
        if let (Some(f), Some(s)) = (i_first, i_second) {
            if f < s {
                let want_second = want_of(&items[s]).unwrap_or(0);
                let cur_first = current(first_key).unwrap_or(0);
                if want_second > cur_first {
                    let second = items.remove(s);
                    items.insert(f, second);
                }
            }
        }
    }
    items
}

fn plan_ops_ordered<'a>(plan: &'a Plan, probe: &ProbeData) -> Vec<&'a PlannedOp> {
    ordered_pairs_with(
        plan.ops.iter().collect::<Vec<_>>(),
        |o| o.key.as_str(),
        |o| o.resolved.parse().ok(),
        |key| probe.entries.get(key).and_then(|e| e.value.clone()).and_then(|v| v.parse().ok()),
    )
}

/// Freq-family read-backs can transiently disagree while an external QoS is
/// active (perf HAL boost holds a higher floor / thermal holds a lower cap).
/// The kernel settles within ~1 s of the QoS expiring, so a single short
/// re-read removes the false failure without a full second apply pass.
fn mismatch_is_transient(kind: catalog::Kind) -> bool {
    matches!(kind, catalog::Kind::Freq | catalog::Kind::FreqMin | catalog::Kind::FreqMax)
}

fn verified_readback(
    key: &str,
    path: &str,
    resolved: &str,
) -> (bool, Option<String>) {
    let kind = catalog::find(key).map(|e| e.kind);
    let check = |back: Option<&str>| match (kind, back) {
        (Some(k), Some(rb)) => readback_matches(k, resolved, rb),
        _ => false,
    };
    let back = probe::read(path);
    if check(back.as_deref()) {
        return (true, None);
    }
    if matches!(kind, Some(k) if mismatch_is_transient(k)) {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let back2 = probe::read(path);
        if check(back2.as_deref()) {
            return (true, None);
        }
        return (
            false,
            Some(format!(
                "read-back mismatch: wrote {resolved} got {}",
                back2.as_deref().unwrap_or("<unreadable>")
            )),
        );
    }
    (
        false,
        Some(format!(
            "read-back mismatch: wrote {resolved} got {}",
            back.as_deref().unwrap_or("<unreadable>")
        )),
    )
}

/// Apply a validated plan: snapshot -> guarded writes -> read-back verify.
/// Returns the report; `report.ok == false` only when a real (non-transient)
/// mismatch or write error occurred.
pub fn apply_plan(store: &Store, plan: &Plan, probe: &ProbeData) -> Result<ApplyReport, String> {
    if !plan.ok {
        return Err(format!("plan rejected: {}", plan.errors.join("; ")));
    }
    let (_, snapshot_created) = ensure_snapshot(store, probe, &plan.ops)?;

    let mut report = ApplyReport {
        mode: "apply".into(),
        profile: Some(plan.profile_id.clone()),
        wrote: 0,
        unchanged: 0,
        verified: 0,
        failed: 0,
        locked: Vec::new(),
        results: Vec::new(),
        ok: true,
        snapshot_created,
        active: Some(plan.profile_id.clone()),
    };

    for op in &plan_ops_ordered(plan, probe) {
        match &op.status {
            crate::profile::OpStatus::Locked(reason) => {
                report.locked.push(LockedKey { key: op.key.clone(), reason: reason.clone() });
                continue;
            }
            crate::profile::OpStatus::Unchanged => {
                report.unchanged += 1;
                continue;
            }
            crate::profile::OpStatus::Ok => {}
        }

        let mut res = WriteResult {
            key: op.key.clone(),
            path: op.path.clone(),
            resolved: op.resolved.clone(),
            before: op.current.clone(),
            written: false,
            verified: false,
            error: None,
        };
        match write_one(&op.path, &op.resolved) {
            Ok(()) => {
                res.written = true;
                report.wrote += 1;
                let (ok, err) = verified_readback(&op.key, &op.path, &op.resolved);
                if ok {
                    res.verified = true;
                    report.verified += 1;
                } else {
                    res.error = err;
                    report.failed += 1;
                    report.ok = false;
                }
            }
            Err(e) => {
                // The node exists but the kernel/SELinux rejected this value
                // on this device (read-only file, value range, avc denial).
                // Report as LOCKED — never silently skipped, never a hard
                // failure: the rest of the profile applied fine.
                report.locked.push(LockedKey {
                    key: op.key.clone(),
                    reason: format!("write rejected: {e}"),
                });
            }
        }
        if res.written {
            report.results.push(res);
        }
    }

    if report.failed == 0 {
        let mut st = store.load_state();
        st.active = Some(plan.profile_id.clone());
        st.updated = now_secs();
        st.last_mode = "apply".into();
        store.save_state(&st)?;
    } else {
        report.active = store.load_state().active;
    }
    Ok(report)
}

/// Apply a profile, then run the automatic pass-2 when the first pass hit
/// nodes that were missing under the previous governor/scheduler: switching
/// `scaling_governor` materializes `policyN/schedutil/*`, switching
/// `io.scheduler` materializes `queue/iosched/*` (cfq tunables).
pub fn apply_with_pass2(
    store: &Store,
    profile: &crate::profile::Profile,
) -> Result<ApplyReport, String> {
    let p = probe::probe();
    let plan = crate::profile::build_plan(profile, &p);
    let mut report = apply_plan(store, &plan, &p)?;

    if report.locked.iter().any(|l| l.reason.contains("node missing")) {
        let p2 = probe::probe();
        let plan2 = crate::profile::build_plan(profile, &p2);
        if plan2.ok {
            let r2 = apply_plan(store, &plan2, &p2)?;
            report.wrote += r2.wrote;
            report.verified += r2.verified;
            report.failed += r2.failed;
            report.results.extend(r2.results);
            report.locked = r2.locked; // fresh truth per key
            report.ok = report.ok && r2.ok;
            if r2.active.is_some() {
                report.active = r2.active;
            }
        }
    }
    Ok(report)
}

/// Restore every snapshotted key to its stock value (engine OFF equivalent).
pub fn restore(store: &Store, probe: &ProbeData) -> Result<ApplyReport, String> {
    let mut snap = store
        .load_snapshot()
        .ok_or_else(|| "no snapshot yet (nothing has been applied)".to_string())?;

    let mut report = ApplyReport {
        mode: "restore".into(),
        profile: None,
        wrote: 0,
        unchanged: 0,
        verified: 0,
        failed: 0,
        locked: Vec::new(),
        results: Vec::new(),
        ok: true,
        snapshot_created: false,
        active: None,
    };

    // Reverse of apply order: min before max is unsafe, so keep the same
    // rank ordering (max rank < min rank => max written first).
    let mut entries: Vec<(String, SnapValue)> = snap.values.clone().into_iter().collect();
    entries.sort_by(|a, b| {
        crate::profile::write_rank(&a.0).cmp(&crate::profile::write_rank(&b.0)).then(a.0.cmp(&b.0))
    });

    // Kernel-validated pairs: decide safe write order against LIVE values
    entries = ordered_pairs_with(
        entries,
        |(k, _)| k.as_str(),
        |(_, sv)| sv.value.parse().ok(),
        |key| probe.entries.get(key).and_then(|e| e.value.clone()).and_then(|v| v.parse().ok()),
    );

    for (key, sv) in entries {
        let current = probe.entries.get(&key).and_then(|e| e.value.clone()).or_else(|| probe::read(&sv.path));
        if current.as_deref() == Some(sv.value.as_str()) {
            report.unchanged += 1;
            continue;
        }
        let mut res = WriteResult {
            key: key.clone(),
            path: sv.path.clone(),
            resolved: sv.value.clone(),
            before: current,
            written: false,
            verified: false,
            error: None,
        };
        match write_one(&sv.path, &sv.value) {
            Ok(()) => {
                res.written = true;
                report.wrote += 1;
                let (ok, err) = verified_readback(&key, &sv.path, &sv.value);
                if ok {
                    res.verified = true;
                    report.verified += 1;
                } else {
                    res.error = err;
                    report.failed += 1;
                    report.ok = false;
                }
            }
            Err(e) => {
                res.error = Some(e);
                report.failed += 1;
                report.ok = false;
            }
        }
        report.results.push(res);
    }

    if report.failed == 0 {
        // Snapshot is consumed: clear it so the next apply re-captures stock.
        snap.values.clear();
        store.save_snapshot(&snap)?;
        let _ = fs::remove_file(store.dir.join("snapshot.json"));
        let mut st = store.load_state();
        st.active = None;
        st.updated = now_secs();
        st.last_mode = "restore".into();
        store.save_state(&st)?;
    }
    Ok(report)
}

/// Compare live values against the plan without writing (drift detection).
pub fn verify_plan(store: &Store, plan: &Plan) -> ApplyReport {
    let mut report = ApplyReport {
        mode: "verify".into(),
        profile: Some(plan.profile_id.clone()),
        wrote: 0,
        unchanged: 0,
        verified: 0,
        failed: 0,
        locked: Vec::new(),
        results: Vec::new(),
        ok: true,
        snapshot_created: false,
        active: store.load_state().active,
    };
    for op in &plan.ops {
        match &op.status {
            crate::profile::OpStatus::Locked(reason) => {
                report.locked.push(LockedKey { key: op.key.clone(), reason: reason.clone() });
                continue;
            }
            crate::profile::OpStatus::Unchanged => report.unchanged += 1,
            crate::profile::OpStatus::Ok => {
                // wanted differs from what the fresh probe saw during planning;
                // verify against a *new* read here to detect drift
                let live = probe::read(&op.path);
                let kind = catalog::find(&op.key).map(|e| e.kind);
                let in_sync = match (kind, live.as_deref()) {
                    (Some(k), Some(rb)) => readback_matches(k, &op.resolved, rb),
                    _ => false,
                };
                if in_sync {
                    report.verified += 1;
                } else {
                    report.failed += 1;
                    report.ok = false;
                    report.results.push(WriteResult {
                        key: op.key.clone(),
                        path: op.path.clone(),
                        resolved: op.resolved.clone(),
                        before: op.current.clone(),
                        written: false,
                        verified: false,
                        error: Some(format!(
                            "drift: live={}",
                            live.as_deref().unwrap_or("<unreadable>")
                        )),
                    });
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("mifinetune-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn json_writes_are_atomic_and_valid() {
        // write_json must leave a valid file and never leave .tmp leftovers
        let dir = tmpdir("atomic");
        let store = Store::new(&dir);
        let st = State { active: Some("game".into()), updated: 7, last_mode: "apply".into() };
        store.save_state(&st).unwrap();
        assert_eq!(store.load_state().active.as_deref(), Some("game"));
        // no stray tmp files in the state dir
        let leftovers: Vec<_> = fs::read_dir(&dir).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn freq_mismatches_are_transient_classified() {
        assert!(mismatch_is_transient(catalog::Kind::Freq));
        assert!(mismatch_is_transient(catalog::Kind::FreqMin));
        assert!(mismatch_is_transient(catalog::Kind::FreqMax));
        assert!(!mismatch_is_transient(catalog::Kind::Int));
        assert!(!mismatch_is_transient(catalog::Kind::Mask));
    }

    #[test]
    fn state_roundtrip() {
        let dir = tmpdir("state");
        let store = Store::new(&dir);
        assert!(store.load_state().active.is_none());
        store.save_state(&State { active: Some("game".into()), updated: 42, last_mode: "apply".into() }).unwrap();
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

    #[test]
    fn migrate_pair_write_order_is_kernel_safe() {
        use crate::profile::{Profile, OpStatus};
        // plan: up=60 down=50 (down-first by rank, up > cur_down?)
        let mut params = std::collections::BTreeMap::new();
        params.insert("kernel.sched_upmigrate".to_string(), "60".to_string());
        params.insert("kernel.sched_downmigrate".to_string(), "50".to_string());
        let prof = Profile { id: "t".into(), label: "T".into(), desc: String::new(), params };

        let mut probe = crate::probe::ProbeData {
            device: Default::default(),
            entries: Default::default(),
            options: Default::default(),
            framework: Default::default(),
        };
        // stock: up=71 down=65 -> want_up(60) <= cur_down(65): down first
        probe.entries.insert(
            "kernel.sched_upmigrate".into(),
            crate::probe::EntryState { exists: true, value: Some("71".into()) },
        );
        probe.entries.insert(
            "kernel.sched_downmigrate".into(),
            crate::probe::EntryState { exists: true, value: Some("65".into()) },
        );
        let plan = crate::profile::build_plan(&prof, &probe);
        assert!(plan.ok, "errors: {:?}", plan.errors);
        let ord = plan_ops_ordered(&plan, &probe);
        let keys: Vec<&str> = ord.iter().map(|o| o.key.as_str()).collect();
        let d = keys.iter().position(|k| *k == "kernel.sched_downmigrate").unwrap();
        let u = keys.iter().position(|k| *k == "kernel.sched_upmigrate").unwrap();
        assert!(d < u, "stock->game must write downmigrate first: {keys:?}");

        // reverse case: cur down=40, want up=60 -> up first
        let mut probe2 = probe.clone();
        probe2
            .entries
            .get_mut("kernel.sched_downmigrate")
            .unwrap()
            .value = Some("40".into());
        let plan2 = crate::profile::build_plan(&prof, &probe2);
        let ord2 = plan_ops_ordered(&plan2, &probe2);
        let keys2: Vec<&str> = ord2.iter().map(|o| o.key.as_str()).collect();
        let d2 = keys2.iter().position(|k| *k == "kernel.sched_downmigrate").unwrap();
        let u2 = keys2.iter().position(|k| *k == "kernel.sched_upmigrate").unwrap();
        assert!(u2 < d2, "up-first when want_up > cur_down: {keys2:?}");
        let _ = OpStatus::Ok;
    }

    #[test]
    fn migrate_pair_invariant_rejects_bad_profile() {
        use crate::profile::Profile;
        let mut params = std::collections::BTreeMap::new();
        params.insert("kernel.sched_upmigrate".to_string(), "60".to_string());
        params.insert("kernel.sched_downmigrate".to_string(), "80".to_string());
        let prof = Profile { id: "bad".into(), label: "B".into(), desc: String::new(), params };
        let plan = crate::profile::build_plan(&prof, &fake_probe_shim());
        assert!(!plan.ok);
        assert!(plan.errors.iter().any(|e| e.contains("must be <")), "errors: {:?}", plan.errors);
    }

    fn fake_probe_shim() -> crate::probe::ProbeData {
        crate::probe::ProbeData {
            device: Default::default(),
            entries: Default::default(),
            options: Default::default(),
            framework: Default::default(),
        }
    }

    #[test]
    fn guard_blocks_restore_writes_to_forbidden_paths() {
        // defense in depth: even a tampered snapshot cannot write framework nodes
        assert!(write_one("/sys/module/msm_performance/parameters/cpu_max_freq", "0").is_err());
        assert!(write_one("/proc/sys/vm/swappiness", "50").is_err());
    }
}
