//! Plan model + build_plan: profile + probe -> ordered, validated plan.
//!
//! Responsibility: per-key status (Ok/Unchanged/Locked), kernel-invariant
//! pair checks, write ordering; hard-fail on unknown/forbidden keys.
//! Non-goals: touching the device (`apply` owns writes).

use crate::engine::catalog::{self, Tier};
use crate::engine::probe::ProbeData;
use crate::engine::profile::Profile;
use crate::engine::readback::{readback_matches, readback_matches_exact};
use crate::engine::validate::validate_value;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpStatus {
    /// Will be written (differs from current).
    Ok,
    /// Already the wanted value.
    Unchanged,
    /// Cannot apply: node missing or value invalid on this device.
    Locked(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedOp {
    pub key: String,
    pub path: String,
    pub tier: Tier,
    pub wanted: String,
    /// Wanted after clamping/validation (e.g. nearest OPP).
    pub resolved: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub status: OpStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub profile_id: String,
    pub label: String,
    pub ops: Vec<PlannedOp>,
    /// Keys rejected as unknown/forbidden — non-empty means the plan is void.
    pub errors: Vec<String>,
    pub ok: bool,
}

/// Ordering of writes: framework-safety first (sysctls), CPU governor/freq
/// last (most impactful; max before min so a raise never reads a stale cap).
pub fn write_rank(key: &str) -> u8 {
    if key.starts_with("vm.") {
        10
    } else if key.starts_with("net.") {
        20
    } else if key.starts_with("kernel.") {
        30
    } else if key.starts_with("workqueue.") {
        40
    } else if key.starts_with("io.") {
        50
    } else if key.starts_with("gpu.") {
        60
    } else if key.starts_with("cpuset.") {
        70
    } else if key.starts_with("stune.") {
        80
    } else if key.contains(".core_ctl.") {
        90
    } else if key.contains(".schedutil.") {
        100
    } else if key.ends_with(".scaling_governor") {
        110
    } else if key.ends_with(".scaling_max_freq") {
        120
    } else if key.ends_with(".scaling_min_freq") {
        130
    } else {
        140
    }
}

pub fn build_plan(profile: &Profile, probe: &ProbeData) -> Plan {
    build_plan_opt(profile, probe, false)
}

/// Same as [`build_plan`] (`exact_freq = true` used by the reconcile pass:
/// leftover FreqMin/FreqMax values are rewritten, not accepted as "external
/// QoS winning").
pub fn build_plan_opt(profile: &Profile, probe: &ProbeData, exact_freq: bool) -> Plan {
    let mut errors = Vec::new();
    let mut ops = Vec::new();

    for (key, wanted) in &profile.params {
        let e = match catalog::find(key) {
            Some(e) => e,
            None => {
                // Distinguish "unknown typo" from "known framework key".
                let hint = if catalog::FORBIDDEN_KEYS.contains(&key.as_str()) {
                    "FORBIDDEN: owned by the MIUI framework"
                } else {
                    "unknown key (not in catalog)"
                };
                errors.push(format!("{key}: {hint}"));
                continue;
            }
        };
        if let Err(msg) = catalog::guard_path(e.path) {
            errors.push(format!("{key}: {msg}"));
            continue;
        }

        let state = probe.entries.get(key);
        let exists = state.map(|s| s.exists).unwrap_or(false);
        let current = state.and_then(|s| s.value.clone());

        let (resolved, problem) = if !exists {
            (
                wanted.clone(),
                Some("node missing on this device".to_string()),
            )
        } else {
            validate_value(e, wanted, probe)
        };

        let status = match problem {
            Some(msg) => OpStatus::Locked(msg),
            None => match &current {
                Some(cur) if exact_freq => {
                    if readback_matches_exact(e.kind, &resolved, cur) {
                        OpStatus::Unchanged
                    } else {
                        OpStatus::Ok
                    }
                }
                Some(cur) if readback_matches(e.kind, &resolved, cur) => OpStatus::Unchanged,
                _ => OpStatus::Ok,
            },
        };

        ops.push(PlannedOp {
            key: key.clone(),
            path: e.path.to_string(),
            tier: e.tier,
            wanted: wanted.clone(),
            resolved,
            current,
            status,
        });
    }

    // Kernel invariant: sched_downmigrate < sched_upmigrate (validated by
    // the kernel on every write, EINVAL otherwise). Both are free params in
    // a profile? enforce the pair before anything reaches the device.
    if let (Some(up), Some(down)) = (
        profile.params.get("kernel.sched_upmigrate"),
        profile.params.get("kernel.sched_downmigrate"),
    ) {
        match (up.trim().parse::<i64>(), down.trim().parse::<i64>()) {
            (Ok(u), Ok(d)) if d < u => {}
            (Ok(u), Ok(d)) => errors.push(format!(
                "kernel.sched pair invalid: downmigrate ({d}) must be < upmigrate ({u})"
            )),
            _ => {} // non-numeric values get caught by per-key validation
        }
    }

    // WALT group migration pair (pct values): sysctl.c registers
    // sched_group_upmigrate with min = downmigrate (extra1) and
    // sched_group_downmigrate with max = upmigrate (extra2) — the kernel
    // rejects down > up, equality is legal (999/999 is a real MIUI branch).
    if let (Some(up), Some(down)) = (
        profile.params.get("kernel.sched_group_upmigrate"),
        profile.params.get("kernel.sched_group_downmigrate"),
    ) {
        match (up.trim().parse::<i64>(), down.trim().parse::<i64>()) {
            (Ok(u), Ok(d)) if d <= u => {}
            (Ok(u), Ok(d)) => errors.push(format!(
                "kernel.sched_group pair invalid: downmigrate ({d}) must be <= upmigrate ({u})"
            )),
            _ => {} // non-numeric values get caught by per-key validation
        }
    }

    // GPU pwrlevel invariant: max_pwrlevel <= min_pwrlevel — the kgsl driver
    // silently clamps `level > min_pwrlevel` to min (kgsl_pwrctrl.c:692),
    // equality is legal (single allowed level).
    if let (Some(maxl), Some(minl)) = (
        profile.params.get("gpu.max_pwrlevel"),
        profile.params.get("gpu.min_pwrlevel"),
    ) {
        match (maxl.trim().parse::<i64>(), minl.trim().parse::<i64>()) {
            (Ok(mx), Ok(mn)) if mx <= mn => {}
            (Ok(mx), Ok(mn)) => errors.push(format!(
                "gpu pwrlevel pair invalid: min_pwrlevel ({mn}) must be >= max_pwrlevel ({mx}) — kgsl would clamp"
            )),
            _ => {}
        }
    }

    ops.sort_by(|a, b| {
        write_rank(&a.key)
            .cmp(&write_rank(&b.key))
            .then(a.key.cmp(&b.key))
    });

    Plan {
        profile_id: profile.id.clone(),
        label: profile.label.clone(),
        ok: errors.is_empty(),
        ops,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testutil::{empty_probe, entry_state, fake_probe, profile};

    #[test]
    fn forbidden_key_voids_plan() {
        let p = profile(&[("vm.swappiness", "50")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(!plan.ok);
        assert!(plan.errors[0].contains("FORBIDDEN"));
    }

    #[test]
    fn unknown_key_voids_plan() {
        let p = profile(&[("gpu.turbo_mode", "1")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(!plan.ok);
        assert!(plan.errors[0].contains("unknown key"));
    }

    #[test]
    fn freq_clamps_to_real_opp() {
        let p = profile(&[("policy0.scaling_max_freq", "1500000")]);
        let plan = build_plan(&p, &fake_probe());
        let op = &plan.ops[0];
        assert_eq!(op.resolved, "1324800"); // nearest real OPP
        assert_eq!(op.status, OpStatus::Ok);
    }

    #[test]
    fn governor_not_available_is_locked() {
        let p = profile(&[("policy0.scaling_governor", "performance")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(matches!(plan.ops[0].status, OpStatus::Locked(_)));
    }

    #[test]
    fn unchanged_detected_with_tabs() {
        let p = profile(&[("net.tcp_rmem", "524288 1048576 5505024")]);
        let plan = build_plan(&p, &fake_probe());
        assert_eq!(plan.ops[0].status, OpStatus::Unchanged);
    }

    #[test]
    fn write_order_is_safe() {
        // sysctls first, governor before max before min
        let p = profile(&[
            ("policy0.scaling_min_freq", "768000"),
            ("policy0.scaling_max_freq", "1804800"),
            ("policy0.scaling_governor", "schedutil"),
            ("vm.dirty_ratio", "30"),
        ]);
        let plan = build_plan(&p, &fake_probe());
        let keys: Vec<&str> = plan.ops.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "vm.dirty_ratio",
                "policy0.scaling_governor",
                "policy0.scaling_max_freq",
                "policy0.scaling_min_freq"
            ]
        );
    }

    #[test]
    fn io_sched_plan_detects_real_switch() {
        // regression: profile wants deadline while live=cfq (bracketed) — the
        // op must be Ok (to write), never Unchanged.
        let p = profile(&[("io.scheduler", "deadline")]);
        let mut probe = fake_probe();
        probe.entries.insert(
            "io.scheduler".into(),
            entry_state(true, "noop deadline [cfq]"),
        );
        let plan = build_plan(&p, &probe);
        assert_eq!(plan.ops[0].status, OpStatus::Ok, "must plan a real write");
    }

    #[test]
    fn stune_boost_range_enforced() {
        let p = profile(&[("stune.top-app.boost", "101")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(
            matches!(plan.ops[0].status, OpStatus::Locked(_)),
            "101 must be locked"
        );

        let p = profile(&[("stune.top-app.boost", "100")]);
        let plan = build_plan(&p, &fake_probe());
        assert_eq!(plan.ops[0].status, OpStatus::Ok);
    }

    #[test]
    fn task_thres_bound_is_cluster_cpu_count() {
        // kernel core_ctl.c store_task_thres: val < num_cpus -> EINVAL
        let p = profile(&[("policy0.core_ctl.task_thres", "4")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(
            matches!(plan.ops[0].status, OpStatus::Locked(_)),
            "4 < 6 must be locked"
        );

        let p = profile(&[("policy0.core_ctl.task_thres", "6")]);
        let plan = build_plan(&p, &fake_probe());
        assert_eq!(plan.ops[0].status, OpStatus::Ok);
    }

    #[test]
    fn min_cpus_preclamps_to_max_cpus() {
        // kernel core_ctl.c store_min_cpus: min(val, max_cpus) — silent clamp
        let p = profile(&[("policy0.core_ctl.min_cpus", "9")]);
        let plan = build_plan(&p, &fake_probe());
        assert_eq!(plan.ops[0].resolved, "6", "pre-clamped to live max_cpus");
        assert_eq!(plan.ops[0].status, OpStatus::Ok);
    }

    #[test]
    fn gpu_pwrlevel_pair_allows_equality() {
        // kgsl_pwrctrl.c:692 clamps level > min_pwrlevel to min; equality OK
        let p = profile(&[("gpu.max_pwrlevel", "4"), ("gpu.min_pwrlevel", "4")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(plan.ok, "max == min must be valid: {:?}", plan.errors);

        let p = profile(&[("gpu.max_pwrlevel", "5"), ("gpu.min_pwrlevel", "3")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(!plan.ok, "max > min must be rejected");
    }

    #[test]
    fn migrate_pair_invariant_rejects_bad_profile() {
        let p = profile(&[
            ("kernel.sched_upmigrate", "60"),
            ("kernel.sched_downmigrate", "80"),
        ]);
        let plan = build_plan(&p, &empty_probe());
        assert!(!plan.ok);
        assert!(
            plan.errors.iter().any(|e| e.contains("must be <")),
            "errors: {:?}",
            plan.errors
        );
    }

    #[test]
    fn group_migrate_pair_invariant_allows_equality_rejects_bad() {
        // sysctl.c registers sched_group_upmigrate with extra1 = downmigrate
        // and sched_group_downmigrate with extra2 = upmigrate — the kernel
        // rejects down > up; equality is legal (post_boot has a 900/900 branch).
        let p = profile(&[
            ("kernel.sched_group_upmigrate", "120"),
            ("kernel.sched_group_downmigrate", "140"),
        ]);
        let plan = build_plan(&p, &empty_probe());
        assert!(!plan.ok);
        assert!(
            plan.errors
                .iter()
                .any(|e| e.contains("must be <= upmigrate")),
            "errors: {:?}",
            plan.errors
        );

        let p = profile(&[
            ("kernel.sched_group_upmigrate", "120"),
            ("kernel.sched_group_downmigrate", "120"),
        ]);
        let plan = build_plan(&p, &empty_probe());
        assert!(plan.ok, "equality must be legal: {:?}", plan.errors);
    }
}
