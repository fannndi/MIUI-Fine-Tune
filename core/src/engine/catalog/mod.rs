//! Parameter catalog — the single source of truth for what MiFineTune may write.
//!
//! Responsibility: types + lookup + the forbidden-path guard.
//! - `entries.rs`   the registry table (keys, paths, kinds, tiers)
//! - `forbidden.rs` framework-owned prefixes/keys — never written
//!
//! Non-goals: probing devices, profile logic, IO.
//!
//! Tiers:
//! - `Free`: no writer found in ROM scripts / perf configs (safe to tune).
//! - `Baseline`: written once by `init.qcom.post_boot.sh` or transiently by
//!   the perf HAL; we set it as a profile baseline and the framework may
//!   overlay it (coexist, never fight).
//! - forbidden: runtime-owned by MIUI (thermal, perf locks, PowerKeeper game
//!   cpuset, LMK/zram, charge, cpu_boost, SELinux). Rejected even if a
//!   profile mentions them — [`guard_path`].

use serde::{Deserialize, Serialize};

mod entries;
mod forbidden;

pub use entries::catalog;
pub use forbidden::{FORBIDDEN_KEYS, FORBIDDEN_PREFIXES};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// No runtime or boot writer in the ROM.
    Free,
    /// Boot/transient baseline — coexist model.
    Baseline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Plain integer (single value, exact read-back). Optional `range`
    /// validates against kernel-confirmed bounds (see Entry.range).
    Int,
    /// core_ctl `min_cpus`: kernel silently clamps to `max_cpus`
    /// (`core_ctl.c store_min_cpus` -> `min(val, state->max_cpus)`) — we
    /// pre-clamp to the live max so read-back matches.
    MinCpus,
    /// Integer written to a per-CPU array node (kernel expands it, e.g.
    /// `core_ctl/busy_up_thres` -> `60 60 60 ...`); verify all equal.
    RepeatInt,
    /// CPU frequency, clamped to the policy's real OPP list.
    Freq,
    /// Frequency cap (`scaling_max_freq`): a read-back **lower** than requested
    /// is accepted as in-sync — an external stricter cap (thermal cooling,
    /// freq-QoS) wins, per the harmony rule. Read-back higher = real drift.
    /// Empirically established 2026-10-07: thermal held gold at 1209600 while
    /// apply wanted 1555200.
    FreqMax,
    /// Frequency floor (`scaling_min_freq`): a read-back **higher** than
    /// requested is accepted as in-sync — an external boost (perf HAL
    /// `msm_performance` cpu_min_freq QoS) wins. Kernel mechanism proven in
    /// `drivers/soc/qcom/msm_performance.c: perf_adjust_notify` →
    /// `CPUFREQ_ADJUST → cpufreq_verify_within_limits(policy, min, max)`
    /// clamps `policy->min` before the store; `show_one(scaling_min_freq,
    /// min)` then reports the clamped value. Verified live 2026-10-07:
    /// with QoS min 1248000 active, writing 576000 read back 1248000, and
    /// after removing the QoS the node returned to the written value by
    /// itself. Lives < want (floor lost) = real drift.
    FreqMin,
    /// GPU pwrlevel index, clamped to `0..level_count-1`.
    PwrLevel,
    /// Governor name, must be in `scaling_available_governors`.
    Gov,
    /// GPU devfreq governor, must be in `available_governors`.
    GpuGov,
    /// Block scheduler token, must be offered by the queue.
    IoSched,
    /// TCP congestion control, must be in `tcp_available_congestion_control`.
    TcpCc,
    /// Space-separated list of integers (e.g. `tcp_rmem`).
    Ints,
    /// CPU bitmask string (`0-5`, `0-7`).
    Mask,
    /// Y/N flag (power_efficient style nodes).
    FlagYN,
    /// Free-form token compared verbatim (fallback).
    Text,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: &'static str,
    pub path: &'static str,
    pub tier: Tier,
    pub kind: Kind,
    /// Scope for value lookup: "policy0" | "policy6" | "" (no clamp source).
    pub scope: &'static str,
    /// Kernel-confirmed inclusive bounds for `Kind::Int`
    /// (source-cited where enforced).
    pub range: Option<(i64, i64)>,
    /// For `core_ctl.task_thres`: validate against the cluster's CPU count —
    /// kernel rejects `val < num_cpus` (`core_ctl.c store_task_thres`).
    pub min_cpus_of: Option<&'static str>,
    /// True for per-scheduler nodes (`/queue/iosched/*`): the directory only
    /// exists while the owning scheduler is active — apply must re-plan them
    /// in pass 2 (same mechanism as the schedutil tunables).
    pub scheduler_dependent: bool,
}

pub fn find(key: &str) -> Option<&'static Entry> {
    catalog().iter().find(|e| e.key == key)
}

/// Returns `Err(reason)` when the path is framework-owned. Every write path
/// passes through here — profiles cannot bypass the catalog.
/// A prefix matches on a path boundary: exact, `/`, or a digit (so
/// `/sys/block/zram` also covers `/sys/block/zram0/disksize`).
pub fn guard_path(path: &str) -> Result<(), String> {
    for p in FORBIDDEN_PREFIXES {
        if path == *p {
            return Err(format!("FORBIDDEN: {path} is owned by the MIUI framework"));
        }
        if let Some(rest) = path.strip_prefix(*p) {
            if rest
                .chars()
                .next()
                .map(|c| c == '/' || c.is_ascii_digit())
                .unwrap_or(false)
            {
                return Err(format!("FORBIDDEN: {path} is owned by the MIUI framework"));
            }
        }
    }
    Ok(())
}

pub fn tier_of(path: &str) -> Option<Tier> {
    find_path(path).map(|e| e.tier)
}

fn find_path(path: &str) -> Option<&'static Entry> {
    catalog().iter().find(|e| e.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_no_forbidden_paths() {
        for e in catalog() {
            guard_path(e.path).unwrap_or_else(|err| panic!("{}: {err}", e.key));
        }
    }

    #[test]
    fn catalog_keys_are_unique() {
        let mut keys: Vec<&str> = catalog().iter().map(|e| e.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), catalog().len());
    }

    #[test]
    fn forbidden_paths_rejected() {
        assert!(guard_path("/sys/module/msm_performance/parameters/cpu_max_freq").is_err());
        assert!(guard_path("/sys/class/thermal/thermal_message/sconfig").is_err());
        assert!(guard_path("/proc/sys/vm/swappiness").is_err());
        assert!(guard_path("/dev/cpuset/game/cpus").is_err());
        assert!(guard_path("/sys/class/power_supply/battery/input_suspend").is_err());
        assert!(guard_path("/sys/block/zram0/disksize").is_err());
        assert!(guard_path("/sys/module/cpu_boost/parameters/input_boost_freq").is_err());
    }

    #[test]
    fn allowed_paths_pass() {
        assert!(guard_path("/proc/sys/vm/vfs_cache_pressure").is_ok());
        assert!(guard_path("/sys/devices/system/cpu/cpufreq/policy0/scaling_governor").is_ok());
        // prefix confusion must not trip: a longer benign path that merely
        // starts like a forbidden one without matching the boundary
        assert!(guard_path("/proc/sys/vm/vfs_cache_pressure2").is_ok());
    }

    #[test]
    fn profile_keys_resolve() {
        for k in [
            "policy0.scaling_governor",
            "gpu.governor",
            "io.scheduler",
            "vm.dirty_ratio",
            "net.tcp_rmem",
            "cpuset.top-app.cpus",
            "stune.top-app.boost",
        ] {
            assert!(find(k).is_some(), "missing catalog entry: {k}");
        }
    }

    #[test]
    fn read_only_kernel_params_are_never_cataloged() {
        // kernel/workqueue.c:294 module_param_named(power_efficient, ..., 0444)
        // — the kernel makes this read-only on every device; it must never
        // re-enter the catalog or any profile.
        assert!(
            find("workqueue.power_efficient").is_none(),
            "workqueue.power_efficient is kernel read-only (0444)"
        );
        // perf HAL transient globals stay forbidden
        assert!(guard_path("/proc/sys/kernel/sched_boost").is_err());
        assert!(guard_path("/sys/devices/system/cpu/cpu0/sched_static_cpu_pwr_cost").is_err());
        // runtime-owned perf HAL nodes (libqti-perfd.so strings) stay forbidden
        for p in [
            "/dev/cpuset/foreground/boost/cpus",
            "/dev/cpu_dma_latency",
            "/sys/kernel/mm/ksm/run",
            "/sys/class/kgsl/kgsl-3d0/force_no_nap",
            "/sys/class/kgsl/kgsl-3d0/force_clk_on",
            "/sys/class/kgsl/kgsl-3d0/force_rail_on",
            "/sys/class/kgsl/kgsl-3d0/idle_timer",
            "/sys/class/mmc_host/mmc0/clk_scaling/enable",
            "/proc/sys/vm/swap_ratio",
        ] {
            assert!(
                guard_path(p).is_err(),
                "{p} must be forbidden (perf HAL runtime-owned)"
            );
        }
    }
}
