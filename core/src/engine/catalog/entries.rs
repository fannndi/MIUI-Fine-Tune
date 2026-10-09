//! The writable catalog table for surya (POCO X3 NFC, MIUI 12).
//!
//! Data only: types + guard live in `mod.rs`, forbidden lists in
//! `forbidden.rs`. Every entry here must pass `guard_path`.

use super::{Entry, Kind, Tier};

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
            scheduler_dependent: false,
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
            scheduler_dependent: false,
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
            scheduler_dependent: false,
        }
    };
}

/// Per-scheduler queue tunable (`/queue/iosched/...`, pass-2 handled).
macro_rules! ei {
    ($key:expr, $path:expr, $tier:expr, $min:expr, $max:expr) => {
        Entry {
            key: $key,
            path: $path,
            tier: $tier,
            kind: Kind::Int,
            scope: "",
            range: Some(($min, $max)),
            min_cpus_of: None,
            scheduler_dependent: true,
        }
    };
}

/// The full writable catalog for surya (POCO X3 NFC, MIUI 12).
pub fn catalog() -> &'static [Entry] {
    use Kind::*;
    use Tier::*;
    static ENTRIES: &[Entry] = &[
        // --- policy0 (Silver cpu0-5) ---
        e!(
            "policy0.scaling_governor",
            "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor",
            Baseline,
            Gov,
            "policy0"
        ),
        e!(
            "policy0.scaling_min_freq",
            "/sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq",
            Baseline,
            FreqMin,
            "policy0"
        ),
        e!(
            "policy0.scaling_max_freq",
            "/sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq",
            Baseline,
            FreqMax,
            "policy0"
        ),
        e!(
            "policy0.schedutil.hispeed_freq",
            "/sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_freq",
            Baseline,
            Freq,
            "policy0"
        ),
        e!(
            "policy0.schedutil.hispeed_load",
            "/sys/devices/system/cpu/cpufreq/policy0/schedutil/hispeed_load",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy0.schedutil.up_rate_limit_us",
            "/sys/devices/system/cpu/cpufreq/policy0/schedutil/up_rate_limit_us",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy0.schedutil.down_rate_limit_us",
            "/sys/devices/system/cpu/cpufreq/policy0/schedutil/down_rate_limit_us",
            Baseline,
            Int,
            ""
        ),
        // Predictive Load: util is floored by WALT's predicted load (early
        // ramp); 0 = reactive. Evidence: perf XML declares the resource
        // (commonresourceconfigs 0x11, unused by perfboostsconfig), the
        // executed moorea post_boot block never writes it, live watch during
        // a game boost stayed 0, device write/readback verified 2026-10-09.
        e!(
            "policy0.schedutil.pl",
            "/sys/devices/system/cpu/cpufreq/policy0/schedutil/pl",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy0.core_ctl.min_cpus",
            "/sys/devices/system/cpu/cpu0/core_ctl/min_cpus",
            Baseline,
            MinCpus,
            "policy0"
        ),
        et!(
            "policy0.core_ctl.task_thres",
            "/sys/devices/system/cpu/cpu0/core_ctl/task_thres",
            Baseline,
            "policy0"
        ), // core_ctl.c: val >= num_cpus
        e!(
            "policy0.core_ctl.busy_up_thres",
            "/sys/devices/system/cpu/cpu0/core_ctl/busy_up_thres",
            Baseline,
            RepeatInt,
            ""
        ),
        e!(
            "policy0.core_ctl.busy_down_thres",
            "/sys/devices/system/cpu/cpu0/core_ctl/busy_down_thres",
            Baseline,
            RepeatInt,
            ""
        ),
        e!(
            "policy0.core_ctl.max_cpus",
            "/sys/devices/system/cpu/cpu0/core_ctl/max_cpus",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy0.core_ctl.offline_delay_ms",
            "/sys/devices/system/cpu/cpu0/core_ctl/offline_delay_ms",
            Baseline,
            Int,
            ""
        ),
        // --- policy6 (Gold cpu6-7) ---
        e!(
            "policy6.scaling_governor",
            "/sys/devices/system/cpu/cpufreq/policy6/scaling_governor",
            Baseline,
            Gov,
            "policy6"
        ),
        e!(
            "policy6.scaling_min_freq",
            "/sys/devices/system/cpu/cpufreq/policy6/scaling_min_freq",
            Baseline,
            FreqMin,
            "policy6"
        ),
        e!(
            "policy6.scaling_max_freq",
            "/sys/devices/system/cpu/cpufreq/policy6/scaling_max_freq",
            Baseline,
            FreqMax,
            "policy6"
        ),
        e!(
            "policy6.schedutil.hispeed_freq",
            "/sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_freq",
            Baseline,
            Freq,
            "policy6"
        ),
        e!(
            "policy6.schedutil.hispeed_load",
            "/sys/devices/system/cpu/cpufreq/policy6/schedutil/hispeed_load",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy6.schedutil.up_rate_limit_us",
            "/sys/devices/system/cpu/cpufreq/policy6/schedutil/up_rate_limit_us",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy6.schedutil.down_rate_limit_us",
            "/sys/devices/system/cpu/cpufreq/policy6/schedutil/down_rate_limit_us",
            Baseline,
            Int,
            ""
        ),
        // same Predictive Load knob for the gold cluster (evidence above)
        e!(
            "policy6.schedutil.pl",
            "/sys/devices/system/cpu/cpufreq/policy6/schedutil/pl",
            Baseline,
            Int,
            ""
        ),
        e!(
            "policy6.core_ctl.min_cpus",
            "/sys/devices/system/cpu/cpu6/core_ctl/min_cpus",
            Baseline,
            MinCpus,
            "policy6"
        ),
        et!(
            "policy6.core_ctl.task_thres",
            "/sys/devices/system/cpu/cpu6/core_ctl/task_thres",
            Baseline,
            "policy6"
        ),
        // --- GPU (Adreno 618, 7 pwrlevels 0..6) ---
        e!(
            "gpu.governor",
            "/sys/class/kgsl/kgsl-3d0/devfreq/governor",
            Baseline,
            GpuGov,
            ""
        ),
        e!(
            "gpu.max_pwrlevel",
            "/sys/class/kgsl/kgsl-3d0/max_pwrlevel",
            Baseline,
            PwrLevel,
            ""
        ),
        e!(
            "gpu.min_pwrlevel",
            "/sys/class/kgsl/kgsl-3d0/min_pwrlevel",
            Baseline,
            PwrLevel,
            ""
        ),
        e!(
            "gpu.default_pwrlevel",
            "/sys/class/kgsl/kgsl-3d0/default_pwrlevel",
            Baseline,
            PwrLevel,
            ""
        ),
        // --- block I/O (UFS sda, single-queue: noop/deadline/cfq) ---
        e!(
            "io.scheduler",
            "/sys/block/sda/queue/scheduler",
            Baseline,
            IoSched,
            ""
        ),
        e!(
            "io.nr_requests",
            "/sys/block/sda/queue/nr_requests",
            Free,
            Int,
            ""
        ),
        e!(
            "io.nomerges",
            "/sys/block/sda/queue/nomerges",
            Free,
            Int,
            ""
        ),
        e!(
            "io.rq_affinity",
            "/sys/block/sda/queue/rq_affinity",
            Free,
            Int,
            ""
        ),
        e!("io.iostats", "/sys/block/sda/queue/iostats", Free, Int, ""),
        // --- v0.4: per-scheduler I/O latency tunables (cfq-iosched.c) ---
        // NOT written by post_boot or any perf XML (verified in ROM v12.0.9 +
        // strings of libqti-perfd/netd) -> Free. Per-CFQ knobs follow the
        // same lifecycle as schedutil tunables: their directory only exists
        // while cfq is the active scheduler (kernel/block/cfq-iosched.c
        // creates it in cfq_init_queue) — handled by the same pass-2 re-plan
        // in apply (see main.rs). `scheduler_dependent: true` per entry.
        ei!(
            "io.cfq.quantum",
            "/sys/block/sda/queue/iosched/quantum",
            Free,
            1,
            64
        ),
        ei!(
            "io.cfq.fifo_expire_sync",
            "/sys/block/sda/queue/iosched/fifo_expire_sync",
            Free,
            0,
            100000
        ),
        ei!(
            "io.cfq.fifo_expire_async",
            "/sys/block/sda/queue/iosched/fifo_expire_async",
            Free,
            0,
            10000
        ),
        ei!(
            "io.cfq.back_seek_max",
            "/sys/block/sda/queue/iosched/back_seek_max",
            Free,
            0,
            1000000
        ),
        ei!(
            "io.cfq.slice_sync",
            "/sys/block/sda/queue/iosched/slice_sync",
            Free,
            1,
            500
        ),
        ei!(
            "io.cfq.slice_async",
            "/sys/block/sda/queue/iosched/slice_async",
            Free,
            1,
            500
        ),
        ei!(
            "io.cfq.slice_idle_us",
            "/sys/block/sda/queue/iosched/slice_idle_us",
            Free,
            0,
            100000
        ),
        ei!(
            "io.cfq.target_latency_us",
            "/sys/block/sda/queue/iosched/target_latency_us",
            Free,
            1,
            1000000
        ),
        ei!(
            "io.cfq.group_idle",
            "/sys/block/sda/queue/iosched/group_idle",
            Free,
            0,
            1
        ),
        ei!(
            "io.cfq.low_latency",
            "/sys/block/sda/queue/iosched/low_latency",
            Free,
            0,
            1
        ),
        // --- v0.7 additions (evidence: pulled ROM V12.0.9 + kernel source) ---
        // read_ahead_kb: init.qcom.rc writes only dm-*/mmcblk* (dm-* at boot,
        // reset to 512 after boot); sda (userdata, f2fs) is never written by
        // any script/XML, and the patched kernel default is VM_MAX_READAHEAD
        // = 512 (include/linux/mm.h:2378, Xiaomi patch) — Free.
        // queue_ra_store sets bdi->ra_pages (block/blk-sysfs.c:101).
        er!(
            "io.read_ahead_kb",
            "/sys/block/sda/queue/read_ahead_kb",
            Free,
            Int,
            "",
            0,
            8192
        ),
        // L3-latency devfreq floors: post_boot writes cpu0/cpu6 l3-lat
        // min/max per SKU (the SA6150 block sets min 940800000); the perf
        // HAL does not touch them at runtime -> Baseline. Values must be in
        // the node's available_frequencies (device-verified list).
        er!(
            "devfreq.cpu0_l3_lat.min_freq",
            "/sys/class/devfreq/soc:qcom,cpu0-cpu-l3-lat/min_freq",
            Baseline,
            Int,
            "",
            0,
            2000000000
        ),
        er!(
            "devfreq.cpu6_l3_lat.min_freq",
            "/sys/class/devfreq/soc:qcom,cpu6-cpu-l3-lat/min_freq",
            Baseline,
            Int,
            "",
            0,
            2000000000
        ),
        // f2fs storage maintenance: /vendor/bin/checkpoint_gc (AOSP, boot)
        // sets gc_urgent_sleep_time=50 + gc_urgent=1, polls dirty_segments
        // until <=100 then restores — boot-transient -> Baseline. Kernel:
        // gc_urgent >= 1 -> GC_URGENT, 0 -> GC_NORMAL (fs/f2fs/sysfs.c:259).
        er!(
            "f2fs.gc_urgent",
            "/sys/fs/f2fs/sda16/gc_urgent",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        er!(
            "f2fs.gc_urgent_sleep_time",
            "/sys/fs/f2fs/sda16/gc_urgent_sleep_time",
            Baseline,
            Int,
            "",
            1,
            10000
        ),
        // v0.7 charge guard: the ROM's init.target.rc explicitly opens this
        // node for userspace (chmod 0777 + chown system) and nothing writes
        // it at runtime — ALLOWED_EXACT overrides the power_supply prefix.
        // Baseline: the JEITA/step-charge path may override on charger
        // events (coexist, never fight).
        er!(
            "charge.battery_charging_enabled",
            "/sys/class/power_supply/battery/battery_charging_enabled",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        // v0.8 per-app bypass: init.target.rc + init.miui.rc open this node
        // for userspace (chmod 0777 + chown system); MIUI's own mishow.sh
        // writes it and hvdcp_opti only reads it (therm-balance monitor).
        // Kernel: vote 0 mA on usb_icl + suspend dc (USER_VOTER) -> input
        // cut, the device runs on battery. Device-verified 2026-10-08.
        er!(
            "charge.input_suspend",
            "/sys/class/power_supply/battery/input_suspend",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        // --- scheduler sysctls (NOT written by the moorea post_boot block) ---
        // WALT damping for the Predictive Load floor (only active while pl=1:
        // predicted load is scaled by TARGET_LOAD). BASELINE: post_boot writes
        // it (1) in the lito/atoll arms — our executed moorea arm (soc
        // 365/366) leaves the kernel default 0, perf XML does not mention it,
        // and a live watch during a game boost stayed 0; device
        // write/readback verified 2026-10-09. Baseline keeps the owner-map
        // audit's static rule ("written by some ROM script") satisfied.
        e!(
            "kernel.sched_conservative_pl",
            "/proc/sys/kernel/sched_conservative_pl",
            Baseline,
            Int,
            ""
        ),
        e!(
            "kernel.sched_latency_ns",
            "/proc/sys/kernel/sched_latency_ns",
            Free,
            Int,
            ""
        ),
        e!(
            "kernel.sched_min_granularity_ns",
            "/proc/sys/kernel/sched_min_granularity_ns",
            Free,
            Int,
            ""
        ),
        e!(
            "kernel.sched_wakeup_granularity_ns",
            "/proc/sys/kernel/sched_wakeup_granularity_ns",
            Free,
            Int,
            ""
        ),
        // perf HAL boost writes this at runtime (commonresourceconfigs.xml
        // Opcode 0x2) — baseline coexist, verified by tools/owner-map-audit.sh
        e!(
            "kernel.sched_migration_cost_ns",
            "/proc/sys/kernel/sched_migration_cost_ns",
            Baseline,
            Int,
            ""
        ),
        e!(
            "kernel.sched_nr_migrate",
            "/proc/sys/kernel/sched_nr_migrate",
            Free,
            Int,
            ""
        ),
        er!(
            "kernel.sched_autogroup_enabled",
            "/proc/sys/kernel/sched_autogroup_enabled",
            Free,
            Int,
            "",
            0,
            1
        ),
        er!(
            "kernel.sched_child_runs_first",
            "/proc/sys/kernel/sched_child_runs_first",
            Free,
            Int,
            "",
            0,
            1
        ),
        // boot-written by the moorea post_boot block -> baseline family
        er!(
            "kernel.sched_walt_rotate_big_tasks",
            "/proc/sys/kernel/sched_walt_rotate_big_tasks",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        er!(
            "kernel.sched_little_cluster_coloc_fmin_khz",
            "/proc/sys/kernel/sched_little_cluster_coloc_fmin_khz",
            Baseline,
            Int,
            "",
            0,
            2000000
        ),
        // migration policy (boot-written by post_boot only — baseline)
        e!(
            "kernel.sched_upmigrate",
            "/proc/sys/kernel/sched_upmigrate",
            Baseline,
            Int,
            ""
        ),
        e!(
            "kernel.sched_downmigrate",
            "/proc/sys/kernel/sched_downmigrate",
            Baseline,
            Int,
            ""
        ),
        e!(
            "kernel.sched_group_upmigrate",
            "/proc/sys/kernel/sched_group_upmigrate",
            Baseline,
            Int,
            ""
        ),
        e!(
            "kernel.sched_group_downmigrate",
            "/proc/sys/kernel/sched_group_downmigrate",
            Baseline,
            Int,
            ""
        ),
        // --- v0.4 additions (igeh evidence: kernel/sysctl.c bounds + writer audit) ---
        // colocation v3 block in post_boot (51/35 default; game branch writes
        // 0/0) AND runtime per-opcode by the perf HAL (commonresourceconfigs
        // minors 0x25/0x26) -> Baseline. Bounds 0..=1000
        // (extra1=&zero, extra2=&one_thousand, kernel/sysctl.c:389).
        er!(
            "kernel.sched_min_task_util_for_boost",
            "/proc/sys/kernel/sched_min_task_util_for_boost",
            Baseline,
            Int,
            "",
            0,
            1000
        ),
        er!(
            "kernel.sched_min_task_util_for_colocation",
            "/proc/sys/kernel/sched_min_task_util_for_colocation",
            Baseline,
            Int,
            "",
            0,
            1000
        ),
        // sync-hint group (XML minor 0x28, runtime) — kernel uses plain
        // proc_dointvec (any count works), we constrain to the 0/1 domain.
        er!(
            "kernel.sched_sync_hint_enable",
            "/proc/sys/kernel/sched_sync_hint_enable",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        // XML minor 0x27 (runtime). Kernel bound: extra1=&two (>=2),
        // extra2=&one_thousand (kernel/sched/walt.c:926 default 1000).
        er!(
            "kernel.sched_many_wakeup_threshold",
            "/proc/sys/kernel/sched_many_wakeup_threshold",
            Baseline,
            Int,
            "",
            2,
            1000
        ),
        // sched_time_avg: proc_dointvec_minmax extra1=&one (>=1 dd)
        // (kernel/sysctl.c; core.c:76 const_debug default 1000ms).
        // Free: no ROM/script writer found in post_boot/perf XML.
        er!(
            "kernel.sched_time_avg_ms",
            "/proc/sys/kernel/sched_time_avg_ms",
            Free,
            Int,
            "",
            1,
            10000
        ),
        // timer_migration_handler, extra1=&zero extra2=&one (kernel/sysctl.c
        // 1357..1363 and the live node accepts the 0..1 domain); post_boot
        // writes "power_aware_timer_migration" which does NOT exist in this
        // kernel (dead write, documented in ROM-HARMONY.md) — timer_migration
        // itself has no ROM writer -> Free.
        er!(
            "kernel.timer_migration",
            "/proc/sys/kernel/timer_migration",
            Free,
            Int,
            "",
            0,
            1
        ),
        // sched_rr_handler (kernel/sched/rt.c:2912): writing <=0 RESETS the
        // timeslice to the default — keep a sane band so a profile never
        // means "reset".
        er!(
            "kernel.sched_rr_timeslice_ms",
            "/proc/sys/kernel/sched_rr_timeslice_ms",
            Free,
            Int,
            "",
            1,
            1000
        ),
        // --- vm (swappiness/min_free_kbytes/page-cluster are forbidden) ---
        e!(
            "vm.vfs_cache_pressure",
            "/proc/sys/vm/vfs_cache_pressure",
            Free,
            Int,
            ""
        ),
        e!("vm.dirty_ratio", "/proc/sys/vm/dirty_ratio", Free, Int, ""),
        e!(
            "vm.dirty_background_ratio",
            "/proc/sys/vm/dirty_background_ratio",
            Free,
            Int,
            ""
        ),
        e!(
            "vm.dirty_expire_centisecs",
            "/proc/sys/vm/dirty_expire_centisecs",
            Free,
            Int,
            ""
        ),
        e!(
            "vm.dirty_writeback_centisecs",
            "/proc/sys/vm/dirty_writeback_centisecs",
            Free,
            Int,
            ""
        ),
        e!(
            "vm.stat_interval",
            "/proc/sys/vm/stat_interval",
            Free,
            Int,
            ""
        ),
        // configure_memory_parameters sets 1 at boot for every target
        // (post_boot: "Disable wsf ... using efk"; range 1..1000 per its own
        // comment) -> boot baseline, audit-tool verified.
        er!(
            "vm.watermark_scale_factor",
            "/proc/sys/vm/watermark_scale_factor",
            Baseline,
            Int,
            "",
            1,
            1000
        ),
        e!(
            "vm.extra_free_kbytes",
            "/proc/sys/vm/extra_free_kbytes",
            Free,
            Int,
            ""
        ),
        // --- net (no ROM writer confirmed) ---
        e!(
            "net.tcp_congestion_control",
            "/proc/sys/net/ipv4/tcp_congestion_control",
            Free,
            TcpCc,
            ""
        ),
        // ConnectivityService applies LinkProperties.TcpBufferSizes (carrier
        // config, visible in `dumpsys connectivity`) through netd's
        // setTcpBufferSizes -> rewrites these on every network re-apply.
        // Empirically observed across display-off cycles (2026-10-07,
        // tools/display-off-diff.sh): the network stack owns them, so they are
        // baseline tier and no shipped profile writes them (profile.rs test).
        e!(
            "net.tcp_rmem",
            "/proc/sys/net/ipv4/tcp_rmem",
            Baseline,
            Ints,
            ""
        ),
        e!(
            "net.tcp_wmem",
            "/proc/sys/net/ipv4/tcp_wmem",
            Baseline,
            Ints,
            ""
        ),
        e!(
            "net.tcp_slow_start_after_idle",
            "/proc/sys/net/ipv4/tcp_slow_start_after_idle",
            Free,
            Int,
            ""
        ),
        e!(
            "net.tcp_mtu_probing",
            "/proc/sys/net/ipv4/tcp_mtu_probing",
            Free,
            Int,
            ""
        ),
        e!(
            "net.tcp_fin_timeout",
            "/proc/sys/net/ipv4/tcp_fin_timeout",
            Free,
            Int,
            ""
        ), // proc_dointvec_jiffies, no minmax
        e!(
            "net.tcp_fastopen",
            "/proc/sys/net/ipv4/tcp_fastopen",
            Free,
            Int,
            ""
        ),
        // workqueue.power_efficient intentionally ABSENT:
        // kernel/workqueue.c:294 module_param(..., 0444) — read-only by kernel
        // on every device; never cataloged, never written.
        // --- schedtune (perf HAL boosts top-app transiently; baseline) ---
        er!(
            "stune.top-app.boost",
            "/dev/stune/top-app/schedtune.boost",
            Baseline,
            Int,
            "",
            0,
            100
        ), // tune.c boost_write: 0..=100
        er!(
            "stune.top-app.prefer_idle",
            "/dev/stune/top-app/schedtune.prefer_idle",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        er!(
            "stune.foreground.boost",
            "/dev/stune/foreground/schedtune.boost",
            Baseline,
            Int,
            "",
            0,
            100
        ),
        er!(
            "stune.foreground.prefer_idle",
            "/dev/stune/foreground/schedtune.prefer_idle",
            Baseline,
            Int,
            "",
            0,
            1
        ),
        // --- cpusets (game/gamelite/vr are PowerKeeper-owned: forbidden) ---
        e!(
            "cpuset.background.cpus",
            "/dev/cpuset/background/cpus",
            Baseline,
            Mask,
            ""
        ),
        e!(
            "cpuset.system-background.cpus",
            "/dev/cpuset/system-background/cpus",
            Baseline,
            Mask,
            ""
        ),
        e!(
            "cpuset.foreground.cpus",
            "/dev/cpuset/foreground/cpus",
            Baseline,
            Mask,
            ""
        ),
        e!(
            "cpuset.top-app.cpus",
            "/dev/cpuset/top-app/cpus",
            Baseline,
            Mask,
            ""
        ),
    ];
    ENTRIES
}
