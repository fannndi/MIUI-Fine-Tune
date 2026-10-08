# ROM harmony — MiFineTune ↔ MIUI 12 (surya, Android 10) ↔ kernel `surya-q-oss`

Node-ownership contract for the POCO X3 NFC. Written from three evidence
sources:

1. **Device live** — `V12.0.7.0.QJGIDXM` (runtime node audit via `miui-ft probe`)
2. **ROM unpacked** — `miui_SURYAGlobal_V12.0.9.0.QJGMIXM_7f83537667_10.0`
   (MIO-KITCHEN) — diffed against the device
3. **Kernel source** — `Xiaomi_Kernel_OpenSource` branch `surya-q-oss` (4.14 qcom)

## Who writes what

Catalog (v0.7+): **89 nodes** (53 Baseline, 36 Free). v0.7 added five
evidence-backed entries: `io.read_ahead_kb` (Free — see the row below), the
two L3-latency devfreq floors (Baseline) and the two f2fs GC maintenance
nodes (Baseline — see the storage-maintenance row).

| Node / parameter | Boot (`init.qcom.post_boot.sh`) | Runtime | Owner | MiFineTune |
|---|---|---|---|---|
| `scaling_governor`, `scaling_min/max_freq`, `schedutil/*` | ✓ block `"365"\|"366"` | perf HAL (transient boost via `msm_performance` lock — **not** direct freq writes) | ROM + perf HAL | **Baseline** (settable as default) |
| `core_ctl/*` | ✓ (min/busy/task/not_preferred/offline_delay) | – | ROM | **Baseline** |
| `sched_up/downmigrate`, `group_*`, `walt_rotate`, `coloc_fmin` | ✓ | – | ROM | **Baseline** |
| `sched_latency/min_granularity/wakeup_granularity` | ✗ | ✗ | – | **Free** |
| `sched_min_task_util_for_boost` (51) / `for_colocation` (35) | ✓ colocation-v3 block (game-boost-off writes 0/0 **transient**) | ✗ on surya (XML only maps opcodes 0x1E/0x1F; no msmsteppe config uses them) | ROM | **Baseline** (bounds 0..=1000, `sysctl.c` extra1=&zero/extra2=&one_thousand) |
| `sched_many_wakeup_threshold` (1000) | ✗ | (XML minor 0x27; **no config** uses it on msmsteppe) | – | **Baseline**-conservative (bounds 2..=1000, `extra1=&two`) |
| `sched_sync_hint_enable` (1) | ✗ | (XML minor 0x28; unused by configs) | – | **Baseline**-conservative (domain 0..=1) |
| `sched_time_avg_ms` (1000) | ✗ | ✗ | – | **Free** (>= 1, `proc_dointvec_minmax extra1=&one`) |
| `timer_migration` (1) | ✗ — post_boot writes `power_aware_timer_migration` which **does not exist** in this kernel (dead write, 4×) | ✗ | – | **Free** (0..=1, `timer_migration_handler`) |
| `sched_rr_timeslice_ms` (100) | ✗ | ✗ | – | **Free** (1..=1000; `sched_rr_handler` resets to default when ≤0) |
| `sched_tunable_scaling` / `sched_conservative_pl` / `sched_cstate_aware` | `conservative_pl` ✓ (REV branch) | ✗ | ROM | **Free** (compared individually; `conservative_pl` Baseline 0..=1) |
| `sched_lib_name` / `sched_lib_mask_force` | ✗ | ✓ **perfd** (libqti-perfd strings; kernel comment: "perfd already configure sched_lib_mask_force to 0xf0") | perf HAL | **Forbidden** + `sched_lib_mask_check` (node absent) |
| `queue/iosched/*` (cfq: `quantum/slice_idle_us/target_latency_us/…`) | ✗ (only `read_ahead_kb` is written) | ✗ | – | **Free** — **per-scheduler**: the dir exists only while cfq is active (pass-2, same as schedutil); game (deadline) does not touch it |
| `sched_migration_cost_ns` | ✗ | ✓ perf HAL (`commonresourceconfigs.xml` Opcode 0x2) | perf HAL | **Baseline** (audit-tool finding) |
| `sched_boost` | ✓ (0 at block end) | ✓ perf HAL | perf HAL | **Forbidden** |
| `vm.*` except swappiness/min_free/page-cluster/watermark_scale | ✗ | ✗ | – | **Free** |
| `vm.swappiness`, `min_free_kbytes`, `page-cluster` | ✓ (`configure_memory_parameters`) | lmkd | ROM | **Forbidden** |
| `vm.watermark_scale_factor` | ✓ set to `1` on every target ("we are using efk"; ROM itself writes range 1..1000) | – | ROM | **Baseline** (audit-tool finding) |
| `net.tcp_rmem/wmem` | ✗ | ✓ **network stack**: ConnectivityService sends `LinkProperties.TcpBufferSizes` (carrier values, visible in `dumpsys connectivity`) to netd → procfs; reset on display-off cycles | framework | **Baseline** (not used by profiles; empirical finding 2026-10-07) |
| other `net.*` (cc/fin_timeout/fastopen/mtu_probing/slow_start) | ✗ | ✗ | – | **Free** |
| `net.core/*` (rmem_max, netdev_max_backlog, …) | ✗ | (stack-adjacent; not audited per node) | framework-ish | **Not cataloged** (safe = untouched) |
| `io.scheduler` / `nr_requests` / `nomerges` / `iostats` / `rq_affinity` | ✗ (only `read_ahead_kb` written) | ✗ | – | **Free** |
| `io.read_ahead_kb` (sda/userdata, queue view) | ✗ for **sda** — `init.qcom.rc` writes `dm-0/1/2` only (2048 during boot, reset to 512 after boot); the 512 sda value is the kernel default (Xiaomi patches `VM_MAX_READAHEAD` to 512, `include/linux/mm.h`) | ✗ | – | **Free** (`queue_ra_store` → `bdi->ra_pages`, `block/blk-sysfs.c`; v0.7 profiles: 128/512/1024) |
| `devfreq cpu0/cpu6 l3-lat min_freq` | ✓ SKU blocks write min/max (SA6150: min 940800000) | ✗ (mem_latency governor votes internally; no sysfs writer) | ROM | **Baseline** (v0.7 game floor 940800000, else stock 300000000; values must be in `available_frequencies`) |
| `f2fs gc_urgent` / `gc_urgent_sleep_time` | ✓ `vendor/bin/checkpoint_gc` (AOSP): sleep=50 + urgent=1, polls `dirty_segments` to ≤100, restores | ✗ | ROM (boot window) | **Baseline** — v0.7 storage maintenance reuses the exact AOSP pattern weekly (charging + idle + 60 s screen-off, bounded 10 min); device run 2026-10-08: `gc 3521 -> 30 dirty segments in 70s`, nodes restored |
| `stune/*/schedtune.*` | ✓ `top-app/prefer_idle` | ✓ perf HAL `top-app` | ROM + perf HAL | **Baseline** |
| `stune/{rt,audio-app}` | ✗ | ✓ audio HAL / RT task framework | framework | **Forbidden** (framework cgroups) |
| `stune.root/…/schedtune.colocate` | ✓ `init.target.rc` (root/bg/sys-bg/fg=0, top-app=1) | ✗ | ROM | **Baseline** (not referenced by profiles; coexist) |
| cpuset `background/system-background/foreground/top-app` | ✓ (bg/system-bg) + `writepid` | ✓ perf HAL + framework API | ROM + PowerKeeper | **Baseline** |
| cpuset `game/gamelite/vr` | ✗ | ✓ PowerKeeper (game mode) | PowerKeeper | **Forbidden** |
| cpuset `audio-app/camera-daemon/restricted` | ✗ (mkdir `init.target.rc`, uid cameraserver) | ✓ cameraserver / audio framework | framework | **Forbidden** |
| GPU `min/max/default_pwrlevel` + `devfreq/min|max_freq` | ✗ | ✓ perf HAL + **thermal cooling** (`thermal-devfreq-0`) | perf HAL + thermal | **Baseline** (careful: two views of the same limiter — see "Drift" below) |
| `gpu.devfreq/min_freq` & `max_freq` (Hz view) | ✗ | ✓ perf HAL (xml:gpu) | perf HAL | **Forbidden** (exact-path guard; the Hz view belongs to the framework) |
| `gpu.devfreq/governor` | ✗ | ✗ | – | **Baseline** — device reality: only `msm-adreno-tz` is accepted by kgsl |
| `thermal_message/*`, cooling devices, `msm_performance/*`, `cpu_boost/*`, charge, zRAM | ✗ | ✓ mi_thermald / perf HAL / micharge | framework | **Forbidden** (path guard) |
| perf HAL runtime-only (`/dev/cpuset/foreground/boost/cpus`, `/dev/cpu_dma_latency`, `/sys/kernel/mm/ksm/*`, kgsl `force_no_nap/clk_on/rail_on/idle_timer`, `mmc0/clk_scaling`, `proc_reclaim`, `swap_ratio`, `/proc/%d/sched_group_id`) | ✗ | ✓ libqti-perfd (OptsHandler) / PowerKeeper | framework | **Forbidden** |
| `workqueue.power_efficient` | – | – | **kernel** (0444 hardcoded) | **Never cataloged** (`kernel/workqueue.c:294`) |

**Cross-version identity (proven):** `post_boot.sh`, all five
`vendor/etc/perf/*.xml`, `powerhint.xml`, and `thermal-{normal,map,tgame}.conf`
are **byte-identical** between `V12.0.7.0.QJGIDXM` (device) and
`V12.0.9.0.QJGMIXM`. The owner map applies to the surya-Q MIUI 12 family.

**Q has no `millet_monitor`** (freeze = PowerKeeper framework API) and no
`cmd game` (MIUI 14 Game Mode API is not here).

**Audit findings (2026-10-07, v0.3):**

- **Perf HAL has its own display off/on events** (`perfboostsconfig.xml` Id
  `0x1040`/`0x1041` → opcode `0x40000000`; `display off` group in
  commonresourceconfigs). **Empirical test with `tools/display-off-diff.sh`**
  (stock, 60 s): the only catalog node that changed at display-off was
  `net.tcp_rmem/wmem` (reset by the network stack, see the table row) — no
  other catalog node was touched; MIUI does not park frequencies at
  display-off beyond the normal thermal path.
- **Definitive runtime-writer list** in `tools/perf-hal-runtime-writers.txt`
  (libqti-perfd strings + XML major groups + `netd`); audit tool v2 fails if
  a Free-tier node collides with it.
- **`schedutil/*` runtime**: the perf HAL XML points at the legacy path
  `/sys/devices/system/cpu/cpufreq/schedutil/*` which **does not exist on
  surya** (only `policy0/policy6`) → those writes fail silently; our schedutil
  tunables are pure Baseline from post_boot.
- **Verified dead code**: `pm2/idle_sleep_mode` is an old msm7630 target
  branch only; `app_setting` + `sched_lib_*` (written by perf HAL) exist
  neither in the OSS kernel nor on the device → nothing to harmonize there.
- **Thermal clamp on freq caps (kind `FreqMax`)**: `scaling_max_freq` counts
  as **in-sync when live ≤ requested** — a stricter external cap (thermal
  cooling / freq-QoS) is thermal winning, per the harmony philosophy. Only
  live > requested (cap lost) is drift and gets repaired. Before this
  semantic (exact match), apply/restore reported false failures while thermal
  was active — real case 2026-10-07: gold held at `1209600` vs requested
  `1555200`.
- **QoS floor on min freq (kind `FreqMin`, v0.4)**: the symmetric case — the
  perf HAL holds `scaling_min_freq` **higher** via `msm_performance` QoS
  during game boost; live ≥ requested = in-sync (framework wins), live <
  requested = drift. Live experiment: wrote `576000` under QoS `1248000` →
  read back `1248000` (previously a false `read-back mismatch` that triggered
  retries); QoS removed → the node returned to the written value by itself.
- **`io.scheduler` read-back = only the bracketed token is active**: the
  offered list `noop deadline [cfq]` — verified v0.3 bug: `deadline` matched
  as an offered token while cfq was active → the game apply **never actually
  wrote the elevator**. Fixed (only `[deadline]` counts) + device test: the
  apply now really moves the elevator and the `iosched` dir changes content
  (cfq↔deadline tunables).
- **`power_aware_timer_migration` = dead write**: post_boot writes it 4×,
  but the node does not exist in the surya 4.14 kernel (same class as
  `workqueue.power_efficient`); the standard `timer_migration` that does
  exist is Free.
- **perfd WALT lib-hint**: `sched_lib_name`/`sched_lib_mask_force` are
  runtime-owned by perfd (libqti-perfd strings + kernel comment in
  drivers/cpufreq/cpufreq.c "perfd already configure sched_lib_mask_force to
  0xf0") → **Forbidden**; `sched_lib_mask_check` does not exist on the device.
- **XML minor opcode exploration (v0.4)**: commonresourceconfigs.xml maps 28
  minor opcodes to `sched_*` nodes (0x19 initial_task_util, 0x21 user_hint,
  0x26 window_stats_policy, 0x29 ravg_window_nr_ticks, etc.) — ALL of those
  nodes are **absent** in the surya 4.14 kernel (mapping for other targets);
  what exists and is cataloged: `min_task_util_for_boost/colocation`,
  `many_wakeup_threshold`, `sync_hint_enable`. On surya, perfboostsconfig.xml
  uses **major opcodes** only (cpufreq/gpu QoS) — sched minors are never
  written at runtime on this device.
- **Framework cgroups (v0.4)**: `cpuset {audio-app,camera-daemon,restricted}`
  and `stune {rt,audio-app}` are managed by init/cameraserver/audio HAL →
  Forbidden. `stune.*.schedtune.colocate` is boot-written by `init.target.rc`
  (top-app=1) → Baseline, not referenced by profiles.

## Kernel invariants (from `surya-q-oss` source, device-verified)

| Node | Source rule | Validator impact |
|---|---|---|
| `kernel.sched_upmigrate` / `sched_downmigrate` | `sched_updown_migrate_handler` (`kernel/sched/core.c:6943`): violating writes are **rolled back + `-EINVAL`** — `margin_up ≤ margin_down` ⇔ `upmigrate ≥ downmigrate` | the pair is validated before write + write order derived from live values (down first, unless `want_up > cur_down`) |
| `core_ctl/task_thres` | `store_task_thres` (`core_ctl.c:154`): `val < num_cpus → -EINVAL` | min = cluster CPU count (silver ≥ 6, gold ≥ 2) |
| `core_ctl/min_cpus` | `store_min_cpus`: `min(val, max_cpus)` — **silent clamp** | pre-clamp to live `max_cpus` so read-back matches |
| `core_ctl/busy_*_thres` | 1 value = broadcast, or exactly `num_cpus` values | kind `RepeatInt` (write 1, verify all elements equal) |
| `gpu.max_pwrlevel` | `kgsl_pwrctrl_max_pwrlevel_store` (`kgsl_pwrctrl.c:692`): `level > min_pwrlevel → level = min_pwrlevel` — **silent clamp** | invariant `max ≤ min` (equality allowed); read-back verify catches the clamp (observed in dev: wrote 4 → read 3) |
| `stune/*/boost` | `boost_write` (`tune.c:619`): `boost < 0 \|\| boost > 100 → -EINVAL` | range 0..=100 |
| `vm.watermark_scale_factor` | `extra1=&one, extra2=&one_thousand` (`kernel/sysctl.c:1648`) | range 1..=1000 |
| `scaling_max_freq` (cap) | thermal cooling/freq-QoS may hold a cap **lower** than requested — not an error | kind `FreqMax`: live ≤ want = in-sync; live > want = drift |
| `scaling_min_freq` (floor) | **symmetric**: `msm_performance` QoS (`perf_adjust_notify` → `CPUFREQ_ADJUST` → `cpufreq_verify_within_limits`, `drivers/soc/qcom/msm_performance.c:256`) holds a floor **higher** — not an error | kind `FreqMin`: live ≥ want = in-sync; live < want = drift. Live experiment 2026-10-07: wrote 576000 under QoS 1248000 → read 1248000; QoS removed → returned to 576000 by itself |
| `sched_many_wakeup_threshold` | `extra1=&two` (`kernel/sysctl.c`) — kernel rejects < 2 | range 2..=1000 |
| `sched_min_task_util_for_boost/colocation` | `extra1=&zero, extra2=&one_thousand` (`kernel/sysctl.c:389`) | range 0..=1000 |
| `sched_rr_timeslice_ms` | `sched_rr_handler` (`kernel/sched/rt.c:2912`): write ≤ 0 = **reset to default** | range 1..=1000 so a profile never means "reset" |
| `queue/iosched/*` | the dir is created/destroyed with the active elevator (`cfq_init_queue` / `deadline_init_queue`) | pass-2 re-plan on "node missing" (same as `schedutil`) |
| `io.scheduler` read-back | `show_one` = offered list with the active token **bracketed** | kind `IoSched`: only the bracketed token counts — **verified v0.3 bug**: an unbracketed offered token false-positived → the `deadline` apply was skipped forever |

## Write order (safety)

`vm → net → kernel → io → gpu → cpuset → stune → core_ctl → schedutil →
governor → scaling_max → scaling_min` — freq max before min; kernel pairs are
reordered per the rules above. Restore uses the same order.

## Drift (who overwrites after apply)

- **Empirical test (touch boost ×12 + 5 s): 0 drift** — all Power Save keys
  survived (gpu cap, cpusets, stune, migration_cost, scaling_max).
- **Display-off (v0.4, catalog 84): 0 of 84 nodes changed** — cfq tunables,
  the sched colocation family, timer_migration, rr_timeslice are not touched
  by the framework while the screen is off.
- Since app-launch/game-boost scenarios were not fully tested, the daemon runs
  a **periodic re-evaluate (15 s)**: the engine skips unchanged keys and only
  rewrites drifted ones (repair decided inside the Rust core).
- Confirmed risk (runtime-writer list): perf HAL boosts write
  `devfreq/min|max_freq` (Hz) — another view of `min|max_pwrlevel` (mapping in
  `kgsl_pwrscale`/`kgsl_pwrctrl.c:839-865`) — plus `core_ctl` min/max_cores
  locks, cpusets, stune, and `msm_performance` QoS; the guard covers them all
  (20 overlapping Baseline entries printed by audit tool v2).
- Display-off cycle (stock): the network stack resets `net.tcp_rmem/wmem`;
  profiles no longer touch them (harmony: do not fight the framework).

## MIUI built-in modes (bridge, v0.5 — Rust implementation)

| MIUI mode | Real state | App write | Notes |
|---|---|---|---|
| Battery saver | `Settings.Global low_power` | root put — **live** ✓ | the MIUI page follows the flag; restore via the bridge hold state machine |
| Performance (hidden sheet) | `persist.sys.aries.power_profile` | **not possible** — SELinux rejects setprop from every su context (shell/run-as/untrusted_app); the hidden dialog cannot open over a locked game | what is written is only the `Settings.System power_mode` mirror (silent); the switch label mentions the limitation |
| Refresh rate (v0.7) | framework `DisplayManager` honors `Settings.System user_refresh_rate` per window | root put — **live** ✓ (verified 2026-10-08: CLI-only 60 caps the panel at 60 Hz during animations while the vendor `persist.vendor.dfps.level` prop stays untouched) | game → 120 Hz, powersave-mapped app → 60 Hz; the user's own value returns on exit (hold/restore). The vendor DFPS prop is NEVER written — settings only |
| Ultra battery saver | `EXTREME_POWER_SAVE_MODE_CHANGED` broadcast (not persistent) | retire: restore + stop + config off | MIUI freezes our service — zero intervention is the goal |
| Split screen / floating window | `GameBoosterService` log `mMultiWindowForegroundPackageName` != 'null' | detected via the daemon's `logcat -b main` stream | the arbiter forces `Balance` over mapping & saver (user verdict); screen-off still wins |
| Game Booster (checker) | `thermal_message/sconfig != 0` | root cat | one conflict notification per game session |

**v0.5 gate**: all app-driven syncs (perf mirror, saver follow, refresh
follow, checker) are active only while **Dynamic Profile is ON** — OFF means
the universal base always wins and no MIUI mode is chased (sleep &
multi-window keep working because they are not app-driven).

The hold/restore cycle is a pure state machine (`core/src/daemon/bridge.rs`,
ported from the Kotlin `MiBridgeState`): first hold captures the user's live
value, release writes it back; while a hold is active the arbiter sees the
USER's value (attribution), never our own write. Holds persist in
`/data/adb/mifinetune/holds.json` (atomic write) so a death mid-hold is
recovered on the next daemon start.

Transition detection: two event tags, `am_resume_activity` and
`am_set_resumed_activity` (some launch paths — monkey/new task — emit only the
second). The seed peek trusts only fresh events (≤60 s) because the events
buffer holds hours of history. The daemon parses `logcat -v epoch`, so
freshness is epoch arithmetic (no timezone/year parsing).

## Verification

```bash
# audit the owner map against any unpacked ROM (run on every ROM/kernel change)
tools/owner-map-audit.sh <unpacked-rom-dir>

# empirical display-off behavior (reads the 87 catalog + framework nodes,
# toggles the screen with the power key, diffs automatically)
tools/display-off-diff.sh 60

# on device
su -c /data/local/tmp/mifinetune/miui-ft probe --json   # read all nodes
su -c /data/local/tmp/mifinetune/miui-ft verify <id>    # drift detection
su -c /data/local/tmp/mifinetune/miui-ft restore        # back to stock
```

**Standing rule**: the engine writes a baseline, it never seizes ownership;
thermal always wins on `scaling_max`; restore returns the snapshot 100%
(including the non-OPP quirk `hispeed 1324600`).
