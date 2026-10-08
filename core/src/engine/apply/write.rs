//! Apply a validated plan: snapshot -> guarded writes -> read-back verify.

use super::store::{ensure_snapshot, Store};
use super::verify::verified_readback;
use super::{now_secs, ApplyReport, LockedKey, WriteResult};
use crate::engine::catalog;
use crate::engine::plan::{build_plan, OpStatus, Plan, PlannedOp};
use crate::engine::probe::{self, ProbeData};

pub(super) fn write_one(op_path: &str, resolved: &str) -> Result<(), String> {
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
pub(super) fn ordered_pairs_with<T, K, W>(
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
            OpStatus::Locked(reason) => {
                report.locked.push(LockedKey { key: op.key.clone(), reason: reason.clone() });
                continue;
            }
            OpStatus::Unchanged => {
                report.unchanged += 1;
                continue;
            }
            OpStatus::Ok => {}
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
    profile: &crate::engine::profile::Profile,
) -> Result<ApplyReport, String> {
    let p = probe::probe();
    let plan = build_plan(profile, &p);
    let mut report = apply_plan(store, &plan, &p)?;

    if report.locked.iter().any(|l| l.reason.contains("node missing")) {
        let p2 = probe::probe();
        let plan2 = build_plan(profile, &p2);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_pair_write_order_is_kernel_safe() {
        use crate::engine::profile::Profile;
        use crate::engine::plan::OpStatus;
        // plan: up=60 down=50 (down-first by rank, up > cur_down?)
        let mut params = std::collections::BTreeMap::new();
        params.insert("kernel.sched_upmigrate".to_string(), "60".to_string());
        params.insert("kernel.sched_downmigrate".to_string(), "50".to_string());
        let prof = Profile { id: "t".into(), label: "T".into(), desc: String::new(), params };

        let mut probe = crate::engine::probe::ProbeData {
            device: Default::default(),
            entries: Default::default(),
            options: Default::default(),
            framework: Default::default(),
        };
        // stock: up=71 down=65 -> want_up(60) <= cur_down(65): down first
        probe.entries.insert(
            "kernel.sched_upmigrate".into(),
            crate::engine::probe::EntryState { exists: true, value: Some("71".into()) },
        );
        probe.entries.insert(
            "kernel.sched_downmigrate".into(),
            crate::engine::probe::EntryState { exists: true, value: Some("65".into()) },
        );
        let plan = build_plan(&prof, &probe);
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
        let plan2 = build_plan(&prof, &probe2);
        let ord2 = plan_ops_ordered(&plan2, &probe2);
        let keys2: Vec<&str> = ord2.iter().map(|o| o.key.as_str()).collect();
        let d2 = keys2.iter().position(|k| *k == "kernel.sched_downmigrate").unwrap();
        let u2 = keys2.iter().position(|k| *k == "kernel.sched_upmigrate").unwrap();
        assert!(u2 < d2, "up-first when want_up > cur_down: {keys2:?}");
        let _ = OpStatus::Ok;
    }

    #[test]
    fn guard_blocks_restore_writes_to_forbidden_paths() {
        // defense in depth: even a tampered snapshot cannot write framework nodes
        assert!(write_one("/sys/module/msm_performance/parameters/cpu_max_freq", "0").is_err());
        assert!(write_one("/proc/sys/vm/swappiness", "50").is_err());
    }
}
