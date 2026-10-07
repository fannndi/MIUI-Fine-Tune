//! Profile loading + plan building (pure validation over a probe snapshot).
//!
//! Responsibility: turn `profiles.json` + [`ProbeData`] into an ordered
//! [`Plan`] with per-key status; hard-fail on unknown/forbidden keys.
//! Non-goals: touching the filesystem for writes (apply owns that).

use crate::catalog::{self, Kind, Tier};
use crate::probe::ProbeData;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub desc: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesFile {
    pub schema: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub rom: String,
    pub profiles: Vec<Profile>,
}

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

/// Validate `wanted` for an entry against live options.
/// Returns `(resolved, None)` or `(raw, Some(reason))` when locked.
pub fn validate_value(e: &catalog::Entry, wanted: &str, probe: &ProbeData) -> (String, Option<String>) {
    let wanted = wanted.trim();
    let locked = |msg: &str| (wanted.to_string(), Some(msg.to_string()));
    match e.kind {
        Kind::Int | Kind::RepeatInt => match wanted.parse::<i64>() {
            Ok(v) if v >= 0 => {
                if let Some(scope) = e.min_cpus_of {
                    // kernel: if (val < state->num_cpus) return -EINVAL
                    let n = probe.options.cluster_cpus.get(scope).copied().unwrap_or(0);
                    if n > 0 && (v as usize) < n {
                        return locked(&format!("must be >= {n} (cluster CPU count, core_ctl.c)"));
                    }
                }
                if let Some((lo, hi)) = e.range {
                    if v < lo || v > hi {
                        return locked(&format!("out of range {lo}..={hi} (kernel bound)"));
                    }
                }
                (v.to_string(), None)
            }
            _ => locked("not a non-negative integer"),
        },
        Kind::MinCpus => match wanted.parse::<i64>() {
            Ok(v) if v >= 0 => {
                // kernel clamps silently: min(val, max_cpus) — pre-clamp so
                // read-back verification matches instead of flagging drift.
                let max = probe.options.core_ctl_max.get(e.scope).copied();
                match max {
                    Some(m) => (v.min(m).to_string(), None),
                    None => (v.to_string(), None),
                }
            }
            _ => locked("not a non-negative integer"),
        },
        Kind::Freq | Kind::FreqMax => {
            let n: u64 = match wanted.parse() {
                Ok(n) => n,
                Err(_) => return locked("not a frequency"),
            };
            match probe.options.freqs.get(e.scope) {
                Some(list) if !list.is_empty() => {
                    let best = list.iter().min_by_key(|f| (**f as i64 - n as i64).unsigned_abs());
                    (best.unwrap().to_string(), None)
                }
                _ => locked("no OPP list for scope"),
            }
        }
        Kind::PwrLevel => match wanted.parse::<usize>() {
            Ok(v) if probe.options.gpu_levels > 0 && v < probe.options.gpu_levels => (v.to_string(), None),
            _ => locked(&format!(
                "pwrlevel out of range 0..{}",
                probe.options.gpu_levels.saturating_sub(1)
            )),
        },
        Kind::Gov => one_of(wanted, probe.options.governors.values().next().map(|v| v.as_slice()), "governor"),
        Kind::GpuGov => one_of(wanted, Some(&probe.options.gpu_governors), "gpu governor"),
        Kind::IoSched => one_of(wanted, Some(&probe.options.io_schedulers), "io scheduler"),
        Kind::TcpCc => one_of(wanted, Some(&probe.options.tcp_cc), "tcp cc"),
        Kind::Ints => {
            let toks: Vec<&str> = wanted.split_whitespace().collect();
            if toks.is_empty() || !toks.iter().all(|t| t.parse::<i64>().map(|v| v >= 0).unwrap_or(false)) {
                return locked("not a list of non-negative integers");
            }
            (toks.join(" "), None)
        }
        Kind::Mask => match validate_mask(wanted, probe.options.cpu_count) {
            Ok(norm) => (norm, None),
            Err(msg) => locked(&msg),
        },
        Kind::FlagYN => match wanted.to_ascii_uppercase().as_str() {
            "Y" | "N" | "0" | "1" => (wanted.to_ascii_uppercase(), None),
            _ => locked("expected Y/N/0/1"),
        },
        Kind::Text => {
            if wanted.is_empty() {
                locked("empty value")
            } else {
                (wanted.to_string(), None)
            }
        }
    }
}

fn one_of(wanted: &str, options: Option<&[String]>, what: &str) -> (String, Option<String>) {
    match options {
        Some(opts) if opts.iter().any(|o| o == wanted) => (wanted.to_string(), None),
        Some(opts) => (
            wanted.to_string(),
            Some(format!("{what} not available (have: {})", opts.join(" "))),
        ),
        None => (wanted.to_string(), Some(format!("{what} options unreadable"))),
    }
}

/// Validate a CPU mask (`0-5`, `0,2,4-7`) against the online CPU count.
pub fn validate_mask(want: &str, cpu_count: usize) -> Result<String, String> {
    let mut parts = Vec::new();
    for tok in want.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            return Err("empty mask token".into());
        }
        let segs: Vec<&str> = tok.split('-').collect();
        let (lo, hi) = match segs.as_slice() {
            [a] => {
                let v: usize = a.parse().map_err(|_| format!("bad cpu '{a}'"))?;
                (v, v)
            }
            [a, b] => {
                let x: usize = a.parse().map_err(|_| format!("bad cpu '{a}'"))?;
                let y: usize = b.parse().map_err(|_| format!("bad cpu '{b}'"))?;
                if x > y {
                    return Err(format!("inverted range {tok}"));
                }
                (x, y)
            }
            _ => return Err(format!("bad mask token '{tok}'")),
        };
        if hi >= cpu_count {
            return Err(format!("cpu {hi} >= cpu_count {cpu_count}"));
        }
        parts.push(if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") });
    }
    Ok(parts.join(","))
}

/// Normalize a read-back value for comparison against `resolved`.
pub fn readback_matches(kind: Kind, resolved: &str, readback: &str) -> bool {
    let rb = readback.trim();
    match kind {
        Kind::Ints => {
            let a: Vec<&str> = resolved.split_whitespace().collect();
            let b: Vec<&str> = rb.split_whitespace().collect();
            a == b
        }
        Kind::IoSched => {
            // read-back is the offered list with the active one bracketed:
            // "noop deadline [cfq]" — the resolved value is a single token.
            rb.split_whitespace()
                .any(|t| t.trim_matches(|c| c == '[' || c == ']') == resolved)
        }
        Kind::RepeatInt => {
            // kernel expands one int to per-cpu array: all must equal wanted
            let want: i64 = match resolved.parse() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let vals: Option<Vec<i64>> = rb.split_whitespace().map(|t| t.parse().ok()).collect();
            match vals {
                Some(v) => !v.is_empty() && v.iter().all(|x| *x == want),
                None => false,
            }
        }
        Kind::FlagYN => rb.eq_ignore_ascii_case(resolved) || matches!((rb, resolved), ("0", "N") | ("N", "0") | ("1", "Y") | ("Y", "1")),
        Kind::FreqMax => {
            // A stricter external cap (thermal cooling / freq-QoS) below the
            // requested cap is thermal winning, not drift. Only live > want
            // (cap lost) must be corrected.
            match (resolved.parse::<i64>(), rb.parse::<i64>()) {
                (Ok(want), Ok(live)) => live <= want,
                _ => rb == resolved,
            }
        }
        _ => rb == resolved,
    }
}

/// Normalize a live value before it is stored in the stock snapshot, so the
/// restore path writes something the node accepts (scheduler active token,
/// whitespace-normalized int lists, ...).
pub fn normalize_snapshot(kind: Kind, raw: &str) -> String {
    let raw = raw.trim();
    match kind {
        Kind::IoSched => {
            // prefer the bracketed active token; else the whole string
            for tok in raw.split_whitespace() {
                if tok.starts_with('[') && tok.ends_with(']') {
                    return tok.trim_matches(|c| c == '[' || c == ']').to_string();
                }
            }
            raw.to_string()
        }
        Kind::Ints => raw.split_whitespace().collect::<Vec<_>>().join(" "),
        Kind::Mask => raw.split_whitespace().collect::<Vec<_>>().join(","),
        Kind::FlagYN => {
            let up = raw.to_ascii_uppercase();
            match up.as_str() {
                "Y" | "1" => "Y".into(),
                "N" | "0" => "N".into(),
                other => other.to_string(),
            }
        }
        _ => raw.to_string(),
    }
}

pub fn build_plan(profile: &Profile, probe: &ProbeData) -> Plan {
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
            (wanted.clone(), Some("node missing on this device".to_string()))
        } else {
            validate_value(e, wanted, probe)
        };

        let status = match problem {
            Some(msg) => OpStatus::Locked(msg),
            None => match &current {
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
    if let (Some(up), Some(down)) = (profile.params.get("kernel.sched_upmigrate"),
                                     profile.params.get("kernel.sched_downmigrate")) {
        match (up.trim().parse::<i64>(), down.trim().parse::<i64>()) {
            (Ok(u), Ok(d)) if d < u => {}
            (Ok(u), Ok(d)) => errors.push(format!(
                "kernel.sched pair invalid: downmigrate ({d}) must be < upmigrate ({u})"
            )),
            _ => {} // non-numeric values get caught by per-key validation
        }
    }

    // GPU pwrlevel invariant: max_pwrlevel <= min_pwrlevel — the kgsl driver
    // silently clamps `level > min_pwrlevel` to min (kgsl_pwrctrl.c:692),
    // equality is legal (single allowed level).
    if let (Some(maxl), Some(minl)) = (profile.params.get("gpu.max_pwrlevel"),
                                       profile.params.get("gpu.min_pwrlevel")) {
        match (maxl.trim().parse::<i64>(), minl.trim().parse::<i64>()) {
            (Ok(mx), Ok(mn)) if mx <= mn => {}
            (Ok(mx), Ok(mn)) => errors.push(format!(
                "gpu pwrlevel pair invalid: min_pwrlevel ({mn}) must be >= max_pwrlevel ({mx}) — kgsl would clamp"
            )),
            _ => {}
        }
    }

    ops.sort_by(|a, b| write_rank(&a.key).cmp(&write_rank(&b.key)).then(a.key.cmp(&b.key)));

    Plan {
        profile_id: profile.id.clone(),
        label: profile.label.clone(),
        ok: errors.is_empty(),
        ops,
        errors,
    }
}

pub fn parse_profiles(json: &str) -> Result<ProfilesFile, String> {
    let file: ProfilesFile = serde_json::from_str(json).map_err(|e| format!("profiles.json: {e}"))?;
    if file.schema != 1 {
        return Err(format!("unsupported profiles schema {}", file.schema));
    }
    if file.profiles.is_empty() {
        return Err("profiles.json has no profiles".into());
    }
    let mut ids = std::collections::HashSet::new();
    for p in &file.profiles {
        if p.id.is_empty() || p.params.is_empty() {
            return Err(format!("profile '{}' is empty", p.id));
        }
        if !ids.insert(p.id.as_str()) {
            return Err(format!("duplicate profile id '{}'", p.id));
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{DeviceInfo, EntryState, FrameworkEvidence, Options};
    use std::collections::BTreeMap;

    fn entry_state(exists: bool, value: &str) -> EntryState {
        EntryState { exists, value: if exists { Some(value.into()) } else { None } }
    }

    fn fake_probe() -> ProbeData {
        let mut entries = BTreeMap::new();
        entries.insert("policy0.scaling_governor".into(), entry_state(true, "schedutil"));
        entries.insert("policy0.scaling_max_freq".into(), entry_state(true, "1804800"));
        entries.insert("gpu.governor".into(), entry_state(true, "msm-adreno-tz"));
        entries.insert("io.scheduler".into(), entry_state(true, "noop deadline [cfq]"));
        entries.insert("vm.dirty_ratio".into(), entry_state(true, "20"));
        entries.insert("cpuset.background.cpus".into(), entry_state(true, "0-5"));
        entries.insert("stune.top-app.boost".into(), entry_state(true, "0"));
        entries.insert("policy0.core_ctl.task_thres".into(), entry_state(true, "8"));
        entries.insert("policy0.core_ctl.min_cpus".into(), entry_state(true, "4"));
        entries.insert("workqueue.power_efficient".into(), entry_state(true, "N"));
        entries.insert("net.tcp_rmem".into(), entry_state(true, "524288\t1048576\t5505024"));

        let mut governors = BTreeMap::new();
        governors.insert("policy0".to_string(), vec!["powersave", "schedutil"].iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let mut freqs = BTreeMap::new();
        freqs.insert("policy0".to_string(), vec![576000, 768000, 1324800, 1804800]);

        ProbeData {
            device: DeviceInfo::default(),
            entries,
            options: Options {
                governors,
                freqs,
                gpu_governors: vec!["msm-adreno-tz".into(), "powersave".into()],
                gpu_levels: 7,
                io_schedulers: vec!["noop".into(), "deadline".into(), "cfq".into()],
                tcp_cc: vec!["cubic".into(), "reno".into()],
                cpu_count: 8,
                cluster_cpus: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("policy0".to_string(), 6usize);
                    m.insert("policy6".to_string(), 2usize);
                    m
                },
                core_ctl_max: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("policy0".to_string(), 6i64);
                    m.insert("policy6".to_string(), 2i64);
                    m
                },
            },
            framework: FrameworkEvidence::default(),
        }
    }

    fn profile(params: &[(&str, &str)]) -> Profile {
        Profile {
            id: "t".into(),
            label: "T".into(),
            desc: String::new(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

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
    fn mask_validated_against_cpu_count() {
        assert!(validate_mask("0-7", 8).is_ok());
        assert!(validate_mask("0-8", 8).is_err());
        assert!(validate_mask("0,2,4-7", 8).is_ok());
        assert!(validate_mask("7-0", 8).is_err());
        assert!(validate_mask("a", 8).is_err());
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
        assert_eq!(keys, vec!["vm.dirty_ratio", "policy0.scaling_governor",
                              "policy0.scaling_max_freq", "policy0.scaling_min_freq"]);
    }

    #[test]
    fn repeat_int_readback_all_equal() {
        assert!(readback_matches(Kind::RepeatInt, "60", "60 60 60 60 60 60"));
        assert!(!readback_matches(Kind::RepeatInt, "60", "60 40 60 60 60 60"));
    }

    #[test]
    fn flagyn_readback_flexible() {
        assert!(readback_matches(Kind::FlagYN, "Y", "Y"));
        assert!(readback_matches(Kind::FlagYN, "N", "0"));
        assert!(!readback_matches(Kind::FlagYN, "Y", "N"));
    }

    #[test]
    fn io_sched_readback_is_token_match() {
        assert!(readback_matches(Kind::IoSched, "cfq", "noop deadline [cfq]"));
        assert!(readback_matches(Kind::IoSched, "deadline", "noop [deadline] cfq"));
        assert!(!readback_matches(Kind::IoSched, "cfq", "noop [deadline]"));
        // plan-time: stock value counts as unchanged for the stock profile
        let _ = OpStatus::Unchanged;
    }

    #[test]
    fn snapshot_normalize_makes_values_node_writable() {
        assert_eq!(normalize_snapshot(Kind::IoSched, "noop deadline [cfq]"), "cfq");
        assert_eq!(normalize_snapshot(Kind::Ints, "524288\t1048576\t5505024"), "524288 1048576 5505024");
        assert_eq!(normalize_snapshot(Kind::Mask, "0-5"), "0-5");
        assert_eq!(normalize_snapshot(Kind::FlagYN, "n"), "N");
        assert_eq!(normalize_snapshot(Kind::Int, " 65 "), "65");
    }

    #[test]
    fn stune_boost_range_enforced() {
        let p = profile(&[("stune.top-app.boost", "101")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(matches!(plan.ops[0].status, OpStatus::Locked(_)), "101 must be locked");

        let p = profile(&[("stune.top-app.boost", "100")]);
        let plan = build_plan(&p, &fake_probe());
        assert_eq!(plan.ops[0].status, OpStatus::Ok);
    }

    #[test]
    fn task_thres_bound_is_cluster_cpu_count() {
        // kernel core_ctl.c store_task_thres: val < num_cpus -> EINVAL
        let p = profile(&[("policy0.core_ctl.task_thres", "4")]);
        let plan = build_plan(&p, &fake_probe());
        assert!(matches!(plan.ops[0].status, OpStatus::Locked(_)), "4 < 6 must be locked");

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
    fn embedded_profiles_parse_and_validate_shape() {
        let json = include_str!("../profiles.json");
        let file = parse_profiles(json).expect("bundled profiles.json must parse");
        assert_eq!(file.profiles.len(), 3);
        let ids: Vec<&str> = file.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["powersave", "balance", "game"]);
        // every key of every profile must exist in the catalog
        for prof in &file.profiles {
            for key in prof.params.keys() {
                assert!(catalog::find(key).is_some(), "{}: unknown key {key}", prof.id);
            }
        }
    }

    #[test]
    fn freq_max_accepts_stricter_external_cap() {
        // thermal cooling holds scaling_max below the requested cap -> thermal
        // wins (harmony rule), reported as in-sync, not as drift.
        assert!(readback_matches(Kind::FreqMax, "1555200", "1209600"));
        assert!(readback_matches(Kind::FreqMax, "1555200", "1555200"));
        assert!(!readback_matches(Kind::FreqMax, "1555200", "1804800"));
        // plain Freq (e.g. scaling_min_freq) stays exact
        assert!(!readback_matches(Kind::Freq, "1555200", "1209600"));
    }

    #[test]
    fn embedded_profiles_do_not_fight_the_network_stack() {
        // ConnectivityService + netd rewrite tcp buffers from the carrier's
        // LinkProperties.TcpBufferSizes on network re-apply (observed across
        // display-off cycles, 2026-10-07). The network stack owns these nodes;
        // profiles must not ship values for them.
        let json = include_str!("../profiles.json");
        let file = parse_profiles(json).expect("parse");
        for p in &file.profiles {
            for k in ["net.tcp_rmem", "net.tcp_wmem"] {
                assert!(
                    !p.params.contains_key(k),
                    "profile {} must not tune {k} (ConnectivityService+netd own it)",
                    p.id
                );
            }
        }
    }
}
