//! Parameter catalog — the single source of truth for what MiFineTune may write.
//!
//! Responsibility: static registry of writable keys, their sysfs/proc paths,
//! value kinds and ownership tier; forbidden-path guard.
//! Non-goals: probing devices, profile logic, IO.
//!
//! Tiers:
//!  - `Free`     : no writer found in ROM scripts / perf configs (safe to tune).
//!  - `Baseline` : written once by `init.qcom.post_boot.sh` or transiently by
//!                 the perf HAL; we set it as a profile baseline and the
//!                 framework may overlay it (coexist, never fight).
//!  - forbidden  : runtime-owned by MIUI (thermal, perf locks, PowerKeeper
//!                 game cpuset, LMK/zram, charge, cpu_boost). Rejected even
//!                 if a profile mentions them — enforced by [`guard_path`].

use serde::{Deserialize, Serialize};

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
}

/// Path prefixes that must never be written — runtime-owned by the framework.
/// Checked against every profile key's resolved path (defense in depth).
pub const FORBIDDEN_PREFIXES: &[&str] = &[
    // perf HAL / perfshielder runtime locks & boosts
    "/sys/module/msm_performance",
    "/sys/module/cpu_boost",
    "/sys/devices/system/cpu/sched_static_cpu_pwr_cost",
    // thermal (mi_thermald, thermal HAL, cooling devices)
    "/sys/class/thermal",
    "/data/vendor/thermal",
    "/sys/class/power_supply",
    "/sys/module/lpm_levels",
    // memory killers / swap (ROM-owned: configure_memory_parameters + lmkd)
    "/proc/sys/vm/swappiness",
    "/proc/sys/vm/min_free_kbytes",
    "/proc/sys/vm/page-cluster",
    "/proc/sys/vm/watermark_boost_factor",
    "/sys/module/process_reclaim",
    "/sys/module/lowmemorykiller",
    "/sys/block/zram",
    // PowerKeeper-owned game cpusets
    "/dev/cpuset/game",
    "/dev/cpuset/gamelite",
    "/dev/cpuset/vr",
    // perf HAL transient boost globals (perfboostsconfig.xml writes these)
    "/proc/sys/kernel/sched_boost",
    "/sys/devices/system/cpu/cpu0/sched_static_cpu_pwr_cost",
    // SELinux / kernel core (never tuning targets)
    "/sys/fs/selinux",
    "/proc/sys/kernel/random",
];

/// Forbidden exact keys (post-boot ROM blocks kept as hard rules even though
/// they are boot-written: they define MIUI's power policy, not a tunable).
pub const FORBIDDEN_KEYS: &[&str] = &[
    "vm.swappiness",
    "vm.min_free_kbytes",
    "vm.page-cluster",
    "vm.watermark_boost_factor",
    "block.read_ahead_kb",
    "cpu.online",
];

macro_rules! e {
    ($key:expr, $path:expr, $tier:expr, $kind:expr, $scope:expr) => {
        Entry {
            key: $key,
            path: $path,
            tier: $tier,
            kind: $kind,
            scope: $scope,
            range: None,
            min_cpus_of: None,
        }
    };
}

/// Entry with a kernel-confirmed inclusive integer range.
macro_rules! er {
    ($key:expr, $path:expr, $tier:expr, $kind:expr, $scope:expr, $min:expr, $max:expr) => {
        Entry {
            key: $key,
            path: $path,
            tier: $tier,
            kind: $kind,
            scope: $scope,
            range: Some(($min, $max)),
            min_cpus_of: None,
        }
    };
}

/// Entry validated against a cluster's CPU count (core_ctl task_thres).
macro_rules! et {
    ($key:expr, $path:expr, $tier:expr, $cluster:expr) => {
        Entry {
            key: $key,
            path: $path,
            tier: $tier,
            kind: Kind::Int,
            scope: "",
            range: None,
            min_cpus_of: Some($cluster),
        }
    };
}

/// The full writable catalog for surya (POCO X3 NFC, MIUI 12).
pub fn catalog() -> &'static [Entry] {
    use Kind::*;
    use Tier::*;
    static ENTRIES: &[Entry] = &[
        // --- policy0 (Silver cpu0-5) ---
        e!("policy0.scaling_governor", "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor", Baseline, Gov, "policy0"),
        e!("policy0.scaling_min_freq", "/sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq", Baseline, Freq, "policy0"),
        e!("policy0.scaling_max_freq", "/sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq", Baseline, Freq, "policy0"),
        e!("policy0.schedutil.hispeed_freq", "/sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_freq", Baseline, Freq, "policy0"),
        e!("policy0.schedutil.hispeed_load", "/sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_load", Baseline, Int, ""),
        e!("policy0.schedutil.up_rate_limit_us", "/sys/devices/system/cpu/cpufreq/policy0/schedutil/up_rate_limit_us", Baseline, Int, ""),
        e!("policy0.schedutil.down_rate_limit_us", "/sys/devices/system/cpu/cpufreq/policy0/schedutil/down_rate_limit_us", Baseline, Int, ""),
        e!("policy0.core_ctl.min_cpus", "/sys/devices/system/cpu/cpu0/core_ctl/min_cpus", Baseline, MinCpus, "policy0"),
        et!("policy0.core_ctl.task_thres", "/sys/devices/system/cpu/cpu0/core_ctl/task_thres", Baseline, "policy0"), // core_ctl.c: val >= num_cpus
        e!("policy0.core_ctl.busy_up_thres", "/sys/devices/system/cpu/cpu0/core_ctl/busy_up_thres", Baseline, RepeatInt, ""),
        e!("policy0.core_ctl.busy_down_thres", "/sys/devices/system/cpu/cpu0/core_ctl/busy_down_thres", Baseline, RepeatInt, ""),
        e!("policy0.core_ctl.max_cpus", "/sys/devices/system/cpu/cpu0/core_ctl/max_cpus", Baseline, Int, ""),
        e!("policy0.core_ctl.offline_delay_ms", "/sys/devices/system/cpu/cpu0/core_ctl/offline_delay_ms", Baseline, Int, ""),
        // --- policy6 (Gold cpu6-7) ---
        e!("policy6.scaling_governor", "/sys/devices/system/cpu/cpufreq/policy6/scaling_governor", Baseline, Gov, "policy6"),
        e!("policy6.scaling_min_freq", "/sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq", Baseline, Freq, "policy6"),
        e!("policy6.scaling_max_freq", "/sys/devices/system/cpu/cpufreq/policy6/scaling_max_freq", Baseline, Freq, "policy6"),
        e!("policy6.schedutil.hispeed_freq", "/sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_freq", Baseline, Freq, "policy6"),
        e!("policy6.schedutil.hispeed_load", "/sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_load", Baseline, Int, ""),
        e!("policy6.schedutil.up_rate_limit_us", "/sys/devices/system/cpu/cpufreq/policy6/schedutil/up_rate_limit_us", Baseline, Int, ""),
        e!("policy6.schedutil.down_rate_limit_us", "/sys/devices/system/cpu/cpufreq/policy6/schedutil/down_rate_limit_us", Baseline, Int, ""),
        e!("policy6.core_ctl.min_cpus", "/sys/devices/system/cpu/cpu6/core_ctl/min_cpus", Baseline, MinCpus, "policy6"),
        et!("policy6.core_ctl.task_thres", "/sys/devices/system/cpu/cpu6/core_ctl/task_thres", Baseline, "policy6"),
        // --- GPU (Adreno 618, 7 pwrlevels 0..6) ---
        e!("gpu.governor", "/sys/class/kgsl/kgsl-3d0/devfreq/governor", Baseline, GpuGov, ""),
        e!("gpu.max_pwrlevel", "/sys/class/kgsl/kgsl-3d0/max_pwrlevel", Baseline, PwrLevel, ""),
        e!("gpu.min_pwrlevel", "/sys/class/kgsl/kgsl-3d0/min_pwrlevel", Baseline, PwrLevel, ""),
        e!("gpu.default_pwrlevel", "/sys/class/kgsl/kgsl-3d0/default_pwrlevel", Baseline, PwrLevel, ""),
        // --- block I/O (UFS sda, single-queue: noop/deadline/cfq) ---
        e!("io.scheduler", "/sys/block/sda/queue/scheduler", Baseline, IoSched, ""),
        e!("io.nr_requests", "/sys/block/sda/queue/nr_requests", Free, Int, ""),
        e!("io.nomerges", "/sys/block/sda/queue/nomerges", Free, Int, ""),
        e!("io.rq_affinity", "/sys/block/sda/queue/rq_affinity", Free, Int, ""),
        e!("io.iostats", "/sys/block/sda/queue/iostats", Free, Int, ""),
        // --- scheduler sysctls (NOT written by the moorea post_boot block) ---
        e!("kernel.sched_latency_ns", "/proc/sys/kernel/sched_latency_ns", Free, Int, ""),
        e!("kernel.sched_min_granularity_ns", "/proc/sys/kernel/sched_min_granularity_ns", Free, Int, ""),
        e!("kernel.sched_wakeup_granularity_ns", "/proc/sys/kernel/sched_wakeup_granularity_ns", Free, Int, ""),
        // perf HAL boost writes this at runtime (commonresourceconfigs.xml
        // Opcode 0x2) — baseline coexist, verified by tools/owner-map-audit.sh
        e!("kernel.sched_migration_cost_ns", "/proc/sys/kernel/sched_migration_cost_ns", Baseline, Int, ""),
        e!("kernel.sched_nr_migrate", "/proc/sys/kernel/sched_nr_migrate", Free, Int, ""),
        er!("kernel.sched_autogroup_enabled", "/proc/sys/kernel/sched_autogroup_enabled", Free, Int, "", 0, 1),
        er!("kernel.sched_child_runs_first", "/proc/sys/kernel/sched_child_runs_first", Free, Int, "", 0, 1),
        // boot-written by the moorea post_boot block -> baseline family
        er!("kernel.sched_walt_rotate_big_tasks", "/proc/sys/kernel/sched_walt_rotate_big_tasks", Baseline, Int, "", 0, 1),
        e!("kernel.sched_little_cluster_coloc_fmin_khz", "/proc/sys/kernel/sched_little_cluster_coloc_fmin_khz", Baseline, Int, ""),
        // migration policy (boot-written by post_boot only — baseline)
        e!("kernel.sched_upmigrate", "/proc/sys/kernel/sched_upmigrate", Baseline, Int, ""),
        e!("kernel.sched_downmigrate", "/proc/sys/kernel/sched_downmigrate", Baseline, Int, ""),
        e!("kernel.sched_group_upmigrate", "/proc/sys/kernel/sched_group_upmigrate", Baseline, Int, ""),
        e!("kernel.sched_group_downmigrate", "/proc/sys/kernel/sched_group_downmigrate", Baseline, Int, ""),
        // --- vm (swappiness/min_free_kbytes/page-cluster are forbidden) ---
        e!("vm.vfs_cache_pressure", "/proc/sys/vm/vfs_cache_pressure", Free, Int, ""),
        e!("vm.dirty_ratio", "/proc/sys/vm/dirty_ratio", Free, Int, ""),
        e!("vm.dirty_background_ratio", "/proc/sys/vm/dirty_background_ratio", Free, Int, ""),
        e!("vm.dirty_expire_centisecs", "/proc/sys/vm/dirty_expire_centisecs", Free, Int, ""),
        e!("vm.dirty_writeback_centisecs", "/proc/sys/vm/dirty_writeback_centisecs", Free, Int, ""),
        e!("vm.stat_interval", "/proc/sys/vm/stat_interval", Free, Int, ""),
        // configure_memory_parameters sets 1 at boot for every target
        // (post_boot: "Disable wsf ... using efk"; range 1..1000 per its own
        // comment) -> boot baseline, audit-tool verified.
        er!("vm.watermark_scale_factor", "/proc/sys/vm/watermark_scale_factor", Baseline, Int, "", 1, 1000),
        e!("vm.extra_free_kbytes", "/proc/sys/vm/extra_free_kbytes", Free, Int, ""),
        // --- net (no ROM writer confirmed) ---
        e!("net.tcp_congestion_control", "/proc/sys/net/ipv4/tcp_congestion_control", Free, TcpCc, ""),
        e!("net.tcp_rmem", "/proc/sys/net/ipv4/tcp_rmem", Free, Ints, ""),
        e!("net.tcp_wmem", "/proc/sys/net/ipv4/tcp_wmem", Free, Ints, ""),
        e!("net.tcp_slow_start_after_idle", "/proc/sys/net/ipv4/tcp_slow_start_after_idle", Free, Int, ""),
        e!("net.tcp_mtu_probing", "/proc/sys/net/ipv4/tcp_mtu_probing", Free, Int, ""),
        e!("net.tcp_fin_timeout", "/proc/sys/net/ipv4/tcp_fin_timeout", Free, Int, ""), // proc_dointvec_jiffies, no minmax
        e!("net.tcp_fastopen", "/proc/sys/net/ipv4/tcp_fastopen", Free, Int, ""),
        // workqueue.power_efficient intentionally ABSENT:
        // kernel/workqueue.c:294 module_param(..., 0444) — read-only by kernel
        // on every device; never cataloged, never written.
        // --- schedtune (perf HAL boosts top-app transiently; baseline) ---
        er!("stune.top-app.boost", "/dev/stune/top-app/schedtune.boost", Baseline, Int, "", 0, 100), // tune.c boost_write: 0..=100
        er!("stune.top-app.prefer_idle", "/dev/stune/top-app/schedtune.prefer_idle", Baseline, Int, "", 0, 1),
        er!("stune.foreground.boost", "/dev/stune/foreground/schedtune.boost", Baseline, Int, "", 0, 100),
        er!("stune.foreground.prefer_idle", "/dev/stune/foreground/schedtune.prefer_idle", Baseline, Int, "", 0, 1),
        // --- cpusets (game/gamelite/vr are PowerKeeper-owned: forbidden) ---
        e!("cpuset.background.cpus", "/dev/cpuset/background/cpus", Baseline, Mask, ""),
        e!("cpuset.system-background.cpus", "/dev/cpuset/system-background/cpus", Baseline, Mask, ""),
        e!("cpuset.foreground.cpus", "/dev/cpuset/foreground/cpus", Baseline, Mask, ""),
        e!("cpuset.top-app.cpus", "/dev/cpuset/top-app/cpus", Baseline, Mask, ""),
    ];
    ENTRIES
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
        for k in ["policy0.scaling_governor", "gpu.governor", "io.scheduler",
                  "vm.dirty_ratio", "net.tcp_rmem", "cpuset.top-app.cpus",
                  "stune.top-app.boost"] {
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
    }
}
