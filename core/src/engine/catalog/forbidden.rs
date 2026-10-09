//! Framework-owned paths and keys — never written, enforced by `guard_path`.
//! Evidence per prefix: docs/ROM-HARMONY.md + tools/perf-hal-runtime-writers.txt.

/// Exact paths explicitly audited and allowed despite a forbidden prefix
/// (catalog evidence required — see docs/ROM-HARMONY.md).
pub const ALLOWED_EXACT: &[&str] = &[
    // init.target.rc chmod 0777 + chown system: the user-facing charge switch
    // is deliberately opened for userspace; no boot script or perf HAL writer
    // exists (audited 2026-10-08). Everything else under the prefix stays
    // forbidden (JEITA/step-charge/current limits are driver-owned).
    "/sys/class/power_supply/battery/battery_charging_enabled",
    // Per-app bypass: init.target.rc + init.miui.rc chmod 0777 + chown
    // system; MIUI's mishow.sh writes it and hvdcp_opti only reads it
    // (device-verified 2026-10-08). JEITA/current nodes stay forbidden.
    "/sys/class/power_supply/battery/input_suspend",
];

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
    // runtime-owned by the perf HAL (strings: libqti-perfd.so, 2026-10-07):
    // boost cpuset, PM QoS latency, KSM, GPU nap/rail/clk holds, storage clkscale
    "/dev/cpuset/foreground/boost",
    "/dev/cpu_dma_latency",
    "/sys/kernel/mm/ksm",
    "/sys/class/kgsl/kgsl-3d0/force_no_nap",
    "/sys/class/kgsl/kgsl-3d0/force_clk_on",
    "/sys/class/kgsl/kgsl-3d0/force_rail_on",
    "/sys/class/kgsl/kgsl-3d0/idle_timer",
    "/sys/class/mmc_host/mmc0/clk_scaling",
    // Qualcomm swap-ratio module param (memory subsystem stays ROM-owned)
    "/proc/sys/vm/swap_ratio",
    // perfd WALT library-hint nodes (strings libqti-perfd.so "0xf0" writer;
    // kernel comment in drivers/cpufreq/cpufreq.c: "perfd already configure
    // sched_lib_mask_force to 0xf0"). Per-app legacy API, never a tunable.
    "/proc/sys/kernel/sched_lib_name",
    "/proc/sys/kernel/sched_lib_mask_force",
    "/proc/sys/kernel/sched_lib_mask_check",
    // perf HAL sched group minor (commonresourceconfigs.xml): node does not
    // exist on surya (XML write fails silently) but stay guarded for other
    // kernels in this family.
    "/proc/sys/kernel/sched_freq_aggregate",
    // GPU devfreq Hz view of the pwrlevel limiter — perf HAL writes these at
    // runtime (tools/perf-hal-runtime-writers.txt, xml:gpu group) and thermal
    // cooling uses the same limiter; pwrlevel entries are our Baseline view.
    "/sys/class/kgsl/kgsl-3d0/devfreq/min_freq",
    "/sys/class/kgsl/kgsl-3d0/devfreq/max_freq",
    // framework-owned cgroups (init.target.rc / cameraserver / audio HAL):
    // camera-daemon dir is created+owned by init (uid cameraserver), rt and
    // audio-app stune groups are populated by the audio/RT task framework.
    "/dev/cpuset/audio-app",
    "/dev/cpuset/camera-daemon",
    "/dev/cpuset/restricted",
    "/dev/stune/rt",
    "/dev/stune/audio-app",
    // SELinux / kernel core (never tuning targets)
    "/sys/fs/selinux",
    "/proc/sys/kernel/random",
    // devfreq: memlat/llcc/bw/busmon families are governor + perf-HAL owned
    // (v0.10 device inventory); the cpu*-cpu-l3-lat devices stay cataloged on
    // purpose, everything else here would double-control live governor votes.
    "/sys/class/devfreq/soc:qcom,cpu-cpu-llcc-bw",
    "/sys/class/devfreq/soc:qcom,cpu-llcc-ddr-bw",
    "/sys/class/devfreq/soc:qcom,cpu0-cpu-llcc-lat",
    "/sys/class/devfreq/soc:qcom,cpu6-cpu-llcc-lat",
    "/sys/class/devfreq/soc:qcom,cpu0-llcc-ddr-lat",
    "/sys/class/devfreq/soc:qcom,cpu6-llcc-ddr-lat",
    "/sys/class/devfreq/soc:qcom,cpu0-cpu-ddr-latfloor",
    "/sys/class/devfreq/soc:qcom,cpu6-cpu-ddr-latfloor",
    "/sys/class/devfreq/soc:qcom,gpubw",
    "/sys/class/devfreq/soc:qcom,kgsl-busmon",
    "/sys/class/devfreq/soc:qcom,npu-npu-ddr-bw",
    "/sys/class/devfreq/soc:qcom,snoc_cnoc_keepalive",
    "/sys/class/devfreq/1d84000.ufshc",
    "/sys/class/devfreq/5000000.qcom,kgsl-3d0",
    "/sys/class/devfreq/aa00000.qcom,vidc1",
    "/sys/class/devfreq/mmc0",
    // cpuidle state gating: nothing in the ROM writes it and blocking deep
    // idle states burns battery for no measured win -> reject surface.
    "/sys/devices/system/cpu/cpu0/cpuidle/",
    "/sys/devices/system/cpu/cpuidle/",
    // written only by the lito/atoll arms and ABSENT on this kernel
    // (ls /proc/sys/kernel has no sched_coloc_* nodes) -> future-proof guard.
    "/proc/sys/kernel/sched_coloc_busy_hyst",
    "/proc/sys/kernel/sched_coloc_downmigrate",
    // charger/parallel/USB power paths are micharge + charger-HW owned;
    // our charge surface stays strictly under /battery.
    "/sys/class/power_supply/main/",
    "/sys/class/power_supply/dc/",
    "/sys/class/power_supply/usb/",
    "/sys/class/power_supply/bms/",
    "/sys/class/power_supply/bq2597x-standalone/",
    "/sys/class/power_supply/pc_port/",
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
    // perfd-owned WALT library-hint API (kernel comment: "perfd already
    // configure sched_lib_mask_force to 0xf0 from user space")
    "kernel.sched_lib_name",
    "kernel.sched_lib_mask_force",
    "kernel.sched_lib_mask_check",
    "kernel.sched_freq_aggregate",
];
