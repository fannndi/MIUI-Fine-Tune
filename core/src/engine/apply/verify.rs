//! Drift detection + read-back verification helpers (read-only).

use super::{ApplyReport, LockedKey, Store, WriteResult};
use crate::engine::catalog;
use crate::engine::plan::{OpStatus, Plan};
use crate::engine::probe;
use crate::engine::readback::readback_matches;

/// Freq-family read-backs can transiently disagree while an external QoS is
/// active (perf HAL boost holds a higher floor / thermal holds a lower cap).
/// The kernel settles within ~1 s of the QoS expiring, so a single short
/// re-read removes the false failure without a full second apply pass.
pub(super) fn mismatch_is_transient(kind: catalog::Kind) -> bool {
    matches!(
        kind,
        catalog::Kind::Freq | catalog::Kind::FreqMin | catalog::Kind::FreqMax
    )
}

pub(super) fn verified_readback(key: &str, path: &str, resolved: &str) -> (bool, Option<String>) {
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
            OpStatus::Locked(reason) => {
                report.locked.push(LockedKey {
                    key: op.key.clone(),
                    reason: reason.clone(),
                });
                continue;
            }
            OpStatus::Unchanged => report.unchanged += 1,
            OpStatus::Ok => {
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

    #[test]
    fn freq_mismatches_are_transient_classified() {
        assert!(mismatch_is_transient(catalog::Kind::Freq));
        assert!(mismatch_is_transient(catalog::Kind::FreqMin));
        assert!(mismatch_is_transient(catalog::Kind::FreqMax));
        assert!(!mismatch_is_transient(catalog::Kind::Int));
        assert!(!mismatch_is_transient(catalog::Kind::Mask));
    }
}
