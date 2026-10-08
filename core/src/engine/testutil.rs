//! Shared test fixtures (cfg(test) only): probe + profile builders.
//!
//! One place for the fake surya device so every engine test builds the same
//! world. Keep this file dependency-free.

use crate::engine::profile::Profile;
use crate::engine::probe::{DeviceInfo, EntryState, FrameworkEvidence, Options, ProbeData};
use std::collections::BTreeMap;

pub fn entry_state(exists: bool, value: &str) -> EntryState {
    EntryState { exists, value: if exists { Some(value.into()) } else { None } }
}

/// Probe with no entries and default options (invariant-only tests).
pub fn empty_probe() -> ProbeData {
    ProbeData {
        device: Default::default(),
        entries: Default::default(),
        options: Default::default(),
        framework: Default::default(),
    }
}

/// The canonical fake device: live values + options matching surya.
pub fn fake_probe() -> ProbeData {
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
    governors.insert(
        "policy0".to_string(),
        vec!["powersave", "schedutil"].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    );
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
                let mut m = BTreeMap::new();
                m.insert("policy0".to_string(), 6usize);
                m.insert("policy6".to_string(), 2usize);
                m
            },
            core_ctl_max: {
                let mut m = BTreeMap::new();
                m.insert("policy0".to_string(), 6i64);
                m.insert("policy6".to_string(), 2i64);
                m
            },
        },
        framework: FrameworkEvidence::default(),
    }
}

pub fn profile(params: &[(&str, &str)]) -> Profile {
    Profile {
        id: "t".into(),
        label: "T".into(),
        desc: String::new(),
        params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

/// Fresh temp dir per test (process-id tagged, removed on entry).
pub fn tmpdir(tag: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mifinetune-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
