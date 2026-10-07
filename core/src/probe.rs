//! Device probing — read-only capture of catalog state + validation options
//! + framework evidence (values MIUI owns and we must never write).
//!
//! Responsibility: reading the live device into [`ProbeData`].
//! Non-goals: deciding what to write (profile/apply own that).

use crate::catalog::{catalog, Entry};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeviceInfo {
    pub device: String,
    pub model: String,
    pub rom: String,
    pub soc_id: String,
    pub kernel: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EntryState {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Options {
    /// policy -> available governors
    pub governors: BTreeMap<String, Vec<String>>,
    /// policy -> available frequencies (ascending, kHz)
    pub freqs: BTreeMap<String, Vec<u64>>,
    pub gpu_governors: Vec<String>,
    /// number of GPU pwrlevels (valid indices 0..n)
    pub gpu_levels: usize,
    /// offered block schedulers (current one marked by caller if needed)
    pub io_schedulers: Vec<String>,
    pub tcp_cc: Vec<String>,
    /// number of online-capable CPUs (0..n usable in masks)
    pub cpu_count: usize,
}

/// Read-only evidence of framework-owned nodes — proves coexistence and
/// gives the UI something to display ("MIUI daemons: running").
#[derive(Debug, Clone, Serialize, Default)]
pub struct FrameworkEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mi_thermald: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perf_hal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perfservice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_sconfig: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msm_perf_locks: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_boost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sched_boost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_cpuset: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeData {
    pub device: DeviceInfo,
    pub entries: BTreeMap<String, EntryState>,
    pub options: Options,
    pub framework: FrameworkEvidence,
}

pub fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn write(path: &str, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("{path}: {e}"))
}

fn tokenize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|t| t.trim_matches(|c| c == '[' || c == ']').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn split_numbers(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

fn getprop(name: &str) -> Option<String> {
    let out = std::process::Command::new("getprop").arg(name).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Parse `/sys/devices/system/cpu/possible` ("0-7") into a count.
fn cpu_count() -> usize {
    read("/sys/devices/system/cpu/possible")
        .and_then(|s| {
            let last = s.split(',').last()?;
            let hi = last.split('-').last()?.parse::<usize>().ok()?;
            Some(hi + 1)
        })
        .unwrap_or(8)
}

pub fn probe() -> ProbeData {
    let mut entries = BTreeMap::new();
    for e in catalog() {
        entries.insert(
            e.key.to_string(),
            EntryState { exists: std::path::Path::new(e.path).exists(), value: read(e.path) },
        );
    }

    let mut governors = BTreeMap::new();
    let mut freqs = BTreeMap::new();
    for policy in ["policy0", "policy6"] {
        let base = format!("/sys/devices/system/cpu/cpufreq/{policy}");
        if let Some(g) = read(&format!("{base}/scaling_available_governors")) {
            governors.insert(policy.to_string(), tokenize(&g));
        }
        if let Some(f) = read(&format!("{base}/scaling_available_frequencies")) {
            let mut v: Vec<u64> = f.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            v.sort_unstable();
            freqs.insert(policy.to_string(), v);
        }
    }

    let gpu_governors = read("/sys/class/kgsl/kgsl-3d0/devfreq/available_governors")
        .map(|s| tokenize(&s))
        .unwrap_or_default();
    let gpu_levels = read("/sys/class/kgsl/kgsl-3d0/gpu_available_frequencies")
        .map(|s| s.split_whitespace().count())
        .unwrap_or(0);
    let io_schedulers = read("/sys/block/sda/queue/scheduler")
        .map(|s| tokenize(&s))
        .unwrap_or_default();
    let tcp_cc = read("/proc/sys/net/ipv4/tcp_available_congestion_control")
        .map(|s| split_numbers(&s))
        .unwrap_or_default();

    let options = Options {
        governors,
        freqs,
        gpu_governors,
        gpu_levels,
        io_schedulers,
        tcp_cc,
        cpu_count: cpu_count(),
    };

    let device = DeviceInfo {
        device: getprop("ro.product.device").unwrap_or_else(|| "unknown".into()),
        model: getprop("ro.product.model").unwrap_or_else(|| "unknown".into()),
        rom: getprop("ro.build.version.incremental").unwrap_or_else(|| "unknown".into()),
        soc_id: read("/sys/devices/soc0/soc_id").unwrap_or_else(|| "unknown".into()),
        kernel: read("/proc/version")
            .map(|v| v.split_whitespace().nth(2).unwrap_or("unknown").to_string())
            .unwrap_or_else(|| "unknown".into()),
    };

    let framework = FrameworkEvidence {
        mi_thermald: getprop("init.svc.mi_thermald"),
        perf_hal: getprop("init.svc.perf-hal-2-0"),
        perfservice: getprop("init.svc.vendor.perfservice"),
        thermal_sconfig: read("/sys/class/thermal/thermal_message/sconfig"),
        msm_perf_locks: read("/sys/module/msm_performance/parameters/cpu_max_freq")
            .map(|v| split_numbers(&v).into_iter().take(2).collect::<Vec<_>>().join(" ")),
        input_boost: read("/sys/module/cpu_boost/parameters/input_boost_freq")
            .map(|v| format!("{v} @{}ms", read("/sys/module/cpu_boost/parameters/input_boost_ms").unwrap_or_default())),
        sched_boost: read("/proc/sys/kernel/sched_boost"),
        game_cpuset: read("/dev/cpuset/game/cpus").or_else(|| Some(String::new())),
    };

    ProbeData { device, entries, options, framework }
}

/// Current value for a catalog entry (live read).
pub fn entry_value(e: &Entry) -> Option<String> {
    read(e.path)
}
