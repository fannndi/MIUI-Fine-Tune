//! Restore: write the stock snapshot back (engine OFF equivalent).

use super::store::Store;
use super::verify::verified_readback;
use super::write::{ordered_pairs_with, write_one};
use super::{now_secs, ApplyReport, SnapValue, WriteResult};
use crate::engine::plan::write_rank;
use crate::engine::probe::{self, ProbeData};
use std::fs;

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
    entries.sort_by(|a, b| write_rank(&a.0).cmp(&write_rank(&b.0)).then(a.0.cmp(&b.0)));

    // Kernel-validated pairs: decide safe write order against LIVE values
    entries = ordered_pairs_with(
        entries,
        |(k, _)| k.as_str(),
        |(_, sv)| sv.value.parse().ok(),
        |key| {
            probe
                .entries
                .get(key)
                .and_then(|e| e.value.clone())
                .and_then(|v| v.parse().ok())
        },
    );

    for (key, sv) in entries {
        let current = probe
            .entries
            .get(&key)
            .and_then(|e| e.value.clone())
            .or_else(|| probe::read(&sv.path));
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
        let _ = fs::remove_file(store.dir().join("snapshot.json"));
        let mut st = store.load_state();
        st.active = None;
        st.updated = now_secs();
        st.last_mode = "restore".into();
        store.save_state(&st)?;
    }
    Ok(report)
}
