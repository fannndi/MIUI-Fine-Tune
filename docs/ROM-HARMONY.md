# ROM harmony — MiFineTune ↔ MIUI 12 (surya, Android 10) ↔ kernel `surya-q-oss`

Node-ownership contract for the POCO X3 NFC. Written from three evidence
sources:

1. **Device live** — `V12.0.7.0.QJGIDXM` (runtime node audit via `miui-ft probe`)
2. **ROM unpacked** — `miui_SURYAGlobal_V12.0.9.0.QJGMIXM_7f83537667_10.0`
   (MIO-KITCHEN) — diffed against the device
3. **Kernel source** — `Xiaomi_Kernel_OpenSource` branch `surya-q-oss` (4.14 qcom)

## Who writes what

Catalog (v0.9+): **94 nodes** (58 Baseline, 36 Free). v0.7–v0.9 added ten
evidence-backed entries: `io.read_ahead_kb` (Free — see the row below), the
two L3-latency devfreq floors (Baseline), the two f2fs GC maintenance nodes,
the two charge nodes, the two schedutil Predictive-Load knobs and
`sched_conservative_pl` (all Baseline — see the rows below).

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
| `battery_charging_enabled` (charge guard, v0.7) | chmod 0777 + chown system in `init.target.rc` (opened for userspace); no value writer | ✗ | – (user-facing switch) | **Baseline** via `ALLOWED_EXACT` — the only `power_supply` path we may write; JEITA/step-charge may override (coexist) |
| `schedutil pl` × policy0/policy6 (v0.9) | perf XML declares the resource (`commonresourceconfigs` opcode 0x11 — no perfboostsconfig entry uses it); the executed **moorea** post_boot arm (soc 365/366) never writes it | ✗ (live watch 0/0 during a game boost, 60 s) | – | **Baseline**. Kernel semantics (`kernel/sched/cpufreq_schedutil.c`): `pl=1` floors util by WALT's *predicted* load (early ramp), `conservative_pl=1` scales that floor by TARGET_LOAD. Profiles: game+boost `pl=1`, others `pl=0` (reactive) |
| `kernel.sched_conservative_pl` (v0.9) | post_boot writes 1 in the **lito/atoll** arms only — moorea leaves the kernel default 0 | ✗ | – | **Baseline** (static rule: some ROM script writes it). Device write/readback verified |
| `thermal_message/*`, cooling devices, `msm_performance/*`, `cpu_boost/*`, other charge nodes, zRAM | ✗ | ✓ mi_thermald / perf HAL / micharge | framework | **Forbidden** (path guard) |
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
| `kernel.sched_group_upmigrate` / `sched_group_downmigrate` | `sysctl.c` registers the pct pair cross-bounded (`upmigrate.extra1 = downmigrate`, `downmigrate.extra2 = upmigrate`); `walt.c walt_proc_update_handler` via `proc_dointvec_minmax` rejects `down > up`, **equality legal** | pair validated (`down ≤ up`) + write order derived from live values; device-verified v0.12.0 (100/85 → 140/120 and back in one apply) |
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
- v0.12.0 round-2 nodes: perf HAL also writes `sched_group_up|downmigrate`,
  `sched_little_cluster_coloc_fmin_khz` and `default_pwrlevel` transiently
  during boosts. Observed once on device (2026-10-09): the first game apply
  read back `downmigrate=85` moments after the write — the next forced apply
  healed it. All four are drift-heal class (watchdog ≤ 30 min).
- `gpu.idle_timer` was re-audited for v0.12.0 and **stays forbidden**:
  libqti-perfd (OptsHandler) writes it at runtime; `adrenoboost` does not
  exist on the stock surya kernel. Neither is cataloged.
- Display-off cycle (stock): the network stack resets `net.tcp_rmem/wmem`;
  profiles no longer touch them (harmony: do not fight the framework).

## MIUI built-in modes (bridge, v0.5 — Rust implementation)

| MIUI mode | Real state | App write | Notes |
|---|---|---|---|
| Battery saver | `Settings.Global low_power` | root put — **live** ✓ | the MIUI page follows the flag; restore via the bridge hold state machine |
| Performance (hidden sheet) | `persist.sys.aries.power_profile` | **not possible** — SELinux rejects setprop from every su context (shell/run-as/untrusted_app); the hidden dialog cannot open over a locked game | what is written is only the `Settings.System power_mode` mirror (silent); the switch label mentions the limitation |
| Refresh rate (v0.16.0, per-app) | `Settings.System user_refresh_rate` **plus MIUI's display-feature API**: `DisplayFeatureManager.setScreenEffect(24, hz)` — the exact call MiSettings' RefreshRateActivity makes (DisplayUtils.java); on this ROM the settings key alone does **not** move the panel (mode is driven by `DisplayFeatureHal` → SDM `SetActiveConfig`). Vendor dfps props are never written | root: `settings put` **+** the tiny embedded `refresh_fps.dex` via `app_process` — **live ✓** device-verified 2026-10-09 (`HandleFpsSwitch: really set fps(120)`, active mode really flips 60↔120; launcher 120 / YouTube 60 / HOME 120 / release 60) | Apps Profile surface: capture the user's `user_refresh_rate` on the first write, apply the app target 120/90/60/30 (`Default` = release), screen-off holds 30 Hz; restored on app exit / Dynamic OFF / Service OFF / crash recovery (`holds.json`). Helper source: `tools/refresh-fps/`, embedded at `core/assets/refresh_fps.dex` |
| Bypass charging (v0.8) | `battery/input_suspend` — `init.target.rc` + `init.miui.rc` open it 0777/chown system; MIUI's `mishow.sh` writes it (retail demo); `hvdcp_opti` only reads it | root write — **live** ✓ (device-verified 2026-10-08: `input_suspend=1` → status `Discharging`, current +142 mA; `0` → `Charging`, -60 mA) | per-app only: engaged while the configured app is in front and the charger is online, released at the floor (default 30 %, +5 hysteresis), on app exit / Service OFF / recovery. Kernel: USER_VOTER votes 0 mA on `usb_icl` + DC suspend |
| Ultra battery saver | `EXTREME_POWER_SAVE_MODE_CHANGED` broadcast (not persistent) | retire: restore + stop + config off | MIUI freezes our service — zero intervention is the goal |
| Split screen / floating window | `GameBoosterService` log `mMultiWindowForegroundPackageName` != 'null' | detected via the daemon's `logcat -b main` stream | the arbiter forces `Balance` over mapping & saver (user verdict); screen-off still wins |
| Game Booster (checker) | `thermal_message/sconfig != 0` | root cat | one conflict notification per game session |

**v0.5 gate**: all app-driven syncs (perf mirror, saver follow, checker and
the Apps Profile software layer) are active only while **Dynamic Profile is
ON** — OFF means the universal base always wins and no MIUI mode is chased
(sleep & multi-window keep working because they are not app-driven).

**Rejected (v0.8)**: the per-app HWUI renderer (`debug.hwui.renderer`). The
app-spawned daemon runs in `u:r:untrusted_app:s0`, where SELinux denies
`property_service { set }` on `debug_prop` (audit-denied 2026-10-08). No
workaround is acceptable under the harmony rules, so the feature was dropped
entirely instead of weakening SELinux.

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

## ROM write-target audit (v0.9)

`tools/rom-write-audit.sh <rom> [--device]` automates the hunt for
"input_suspend-like gems":

1. Extract every write target from the ROM: `write /sys|/proc` (init `.rc`)
   + `echo VALUE > /sys|/proc` (`.sh`) → **546 unique paths**.
2. Classify each: `CATALOG` (91 covered pre-v0.9) / `RUNTIME` / `FORBIDDEN`
   / `NEW` (429 candidates).
3. `--device`: probe existence as **root** (plain shell uid loses ~108
   permission-gated `/proc/sys` nodes — 205 of 546 actually exist; the rest
   are dead multi-SoC paths) and run a **live-vs-boot value diff**: a live
   value outside the ROM's boot-value set = unknown runtime writer → never
   Free. Result: 113 same, 341 not-present, 92 value-diff, **0 true unknown
   writers after analysis**.

Findings that corrected earlier assumptions:

| Suspect | Verdict |
|---|---|
| `net.core/rmem_max` (boot 262144 ≠ live 16777216) | **not** a runtime writer — `netmgrd.rc` writes 16777216 at boot too (two writers) |
| `cpu*/sched_load_boost` (boot −6 ≠ live 0 on cpu0–5) | branch semantics: the moorea arm writes −6 only for cpu6/cpu7; cpu0–5 keep the kernel default |
| `vm/swap_ratio` (boot 70 ≠ live 100) | confirmed **runtime writer** (already in the writers list) |
| `tp_palm`, `tp_grip_area` | **read-only (0444)** on device — unwritable, rejected |
| `dsi_display_hbm`/`cabc` | property-triggered by MIUI (`on property:sys.dsi_display_hbm`) → framework controller, rejected |
| `big_cluster_min_freq_adjust`, `power_aware_timer_migration`, most `sched_*` | **absent on device** (other-SoC arms), rejected |
| `battery/charging_enabled` | ROM's write is commented out (would be Free) — node accepted 1→0→1 writes but status showed no effect at 100 %; not cataloged (no consumer) |

Intel: perfd's `libgameoptfeature` (`vendor/etc/lm/GameOptimizationFeature.xml`)
monitors `measured_fps` + mem-lat `cur_freq`, votes **DDR min 1144→2086 MHz**
on workload hints and applies a "Predictive Load 1" game profile — we never
write `llcc-ddr` nodes (we only own the l3-lat floors) and `pl` stays
Baseline/coexist.

Branch caveat: post_boot is a multi-SoC script; a static scan cannot know
which arm executes. Tier decisions therefore use the *executed* arm (soc
365/366 moorea) when known, and the conservative static label otherwise.

## Deep inventory + second audit loop (v0.10)

Beyond ROM write-targets (v0.9), this pass inventoried every readable node on
the device and diffed it against the catalog + forbidden + writers lists:

| Domain | Nodes | Verdict |
|---|---|---|
| `/proc/sys` | 4 492 | kernel/ 510, vm/ 28, net/ 414 surveyed; `sched_coloc_busy_hyst*`/`coloc_downmigrate` are absent on this kernel (lito/atoll arms only) -> forbidden markers added |
| `/sys/module/*/parameters` | 433 | all covered by existing forbidden families (lowmemorykiller, process_reclaim, cpu_boost, wlan, msm…) |
| `/sys/kernel` | ~1 200 | binder stats live only in debugfs; PSI (`/proc/pressure`) absent on kernel 4.14; cpuidle `stateN/disable` exists (C0/C1/C2) with **no ROM writer** -> rejected surface (blocking deep idle burns battery for no measured win) |
| `/sys/devices/system/cpu` | 866 | `scaling_boost_frequencies` is 0444 read-only -> reject; per-policy/alias handling unchanged |
| `/sys/class/devfreq` | 13 devices | memlat/llcc/bw/npu/busmon/keepalive/ufshc/GPU-devfreq/mmc0/vidc are governor- or perf-HAL-owned -> **forbidden prefixes added**; the `cpu*-cpu-l3-lat` devices remain cataloged on purpose |
| power_supply | battery + 5 sub-supplies | charger paths (`main`, `dc`, `usb`, `bms`, `bq2597x-standalone`, `pc_port`) are micharge/HW-owned -> forbidden; battery surface stays under `/battery` |

New read-only diagnostics (no writer anywhere, 0444/0400 root-only):

- `battery/current_now` µA — **negative = charging, positive = discharging**,
  sign verified live via a controlled `input_suspend` bypass test (suspend:
  +600 mA discharge; restore: negative values). 0444 system:system.
- `battery/voltage_now` µV, `cpuinfo_cur_freq` kHz (policy0/6, 0400 root),
  kgsl `gpuclk` Hz (0664), f2fs `lifetime_write_kbytes` (0444; 53 GB total).
  All feed `EnvSnapshot` -> env events -> Diagnostics + doctor (`env_power`,
  `env_freq`).

`sched_boost`: observed `1` during an app-launch burst then `0` (clears
itself) — the perf-HAL transient boost globals stay **forbidden** and the
catalog never registers or writes them; the forbidden entry now carries live
evidence. `sched_cstate_aware` (fair.c idle-sibling placement, kernel default
1, no writer) was surveyed and left out: a placement trade without a
consumable profile value — documented here, not cataloged.

## Profile identities (v0.11.1) — "masih tidak ada bedanya" diagnostic

Audited truth: `balance` shipped by F13 was **byte-identical to the MIUI
moorea post-boot defaults** (19/19 freq/sched/core_ctl/IO/VM keys SAME) —
so daily stock-MIUI behavior plus our balance profile did not feel
different BY DESIGN. This pass gives each profile a real personality
without breaking the harmony rule book:

| key | powersave | balance | game | sleep | boost |
|---|---|---|---|---|---|
| `p0.scaling_governor` | powersave | schedutil | schedutil | schedutil | schedutil |
| `p6.scaling_governor` | powersave | schedutil | schedutil | schedutil | schedutil |
| `p0.scaling_max_freq` | **1248000** | 1804800 | 1804800 | **1017600** | 1804800 |
| `p6.scaling_max_freq` | **1324800** | 2304000 | 2304000 | **1324800** | **2304000** (add) |
| `p0.schedutil.hispeed_freq` | – | **1324800 @ 85** | 1497600 @ 75 | 1017600 @ 95 | 1497600 @ 75 |
| `p6.schedutil.hispeed_freq` | – | **1324800** | 1555200 | 1209600 | 1555200 |
| `p0.core_ctl.min_cpus` | 2 | **2** | 6 | 1 | 6 |
| `gpu.max_pwrlevel` | **5** | 0 | 0 | 5 | – |

Choices and their intent:
1. **balance is now "stock-plus"** — the two Micro knobs the user feels
   (`hispeed_freq` 1248→1324800 with `hispeed_load` 90→85 mean a swipe
   reaches 1324 MHz instead of 1248) plus `core_ctl.min_cpus 4→2`
   (two little cores park at idle, wake latency is trivial). Battery
   should outperform stock slightly and UI feel slightly snappier.
2. **powersave truthfully saves**: both governors are `powersave`, and
   the clocks/GPU are capped (1248 / 1324 / gpu level 5). When the user
   picks this card the phone will actually be slower on heavy tasks —
   that expectation is documented.
3. **sleep** caps below the own hispeed values so background bursts do
   not run hot while the screen is off (1017600 / 1324800).
4. **boost fixes a real gap**: a jank burst with a powersave or sleep
   base previously *could not* exceed the capped `p6.max` because the
   boost profile never wrote it. Now the 5-s window writes 2304000 and
   always leaves the device at full-cap CPU (1804800 / 2304000) for the
   snap.
5. **game is unchanged on purpose**: the values remaining after F13's
   bench are the tuned ceiling (1094400 floor, 1555200 hispeed,
   1804800 little max, `deadline` scheduler, 1024 read-ahead, GPU floor
   level 3, stune boost 10 + prefer_idle, pl=1).

Ownership audit still reports 94 catalog entries / 0 violations; every
tuned value is inside the device OPP list or a kernel-trusted clamp.

## Auto-revive + hands-off (v0.11)

**Symptom found live**: a session's tail values (from earlier applies or a
crashed daemon mid-apply) survive profile switches because the periodic
fast-path only reflects the *current* profile's keys — `p6.min_freq`
stayed at the game/boost floor (1094400), `p0.max` at the sleep cap
(1248000), `vm.stat_interval` at the sleep value after the base re-applied.
The user experienced this as "frequency locked, cannot drop".

Three-part fix:

1. **`stock.json` (persistent union-stock)** — one entry per profile-union
   key; seeded once at first apply (prefer the legacy snapshot if present,
   else the live device at that moment; on surya the seeder table is
   `moorea post_boot` values + kernel defaults). Grows monotonically as
   profiles introduce new keys; never consumed.
2. **Full-reconcile pass (force applies)** — every forced apply
   (`config`/`profiles` change, first apply of a daemon run, user tap, and
   the watchdog) merges the active profile with the stock map: keys the
   profile *does* set take the profile value; every *other* union key gets
   its stock value back. FreqMin/FreqMax are compared **exactly** in this
   pass (`readback_matches_exact`): harmony still relaxes to thermal/QoS
   in the periodic fast path, but a reconcile must rewrite a leftover
   valve. The coalescing worker carries `force` sticky across a batch so
   the startup screen-on + unlock race cannot swallow it.
3. **Hands-off service-off** — on `restore` (service OFF / MIUI ultra
   saver retire) the daemon releases every bridge artifact it holds and
   then writes *nothing*: `active = null`, `last_mode = "hands-off"` —
   the user explicitly asked for "matikan service = lepas intervensi".
   The catalog nodes keep their last state (MIUI's own controllers and a
   reboot reset them); `miui-ft restore` remains the explicit CLI
   back-to-stock repair tool (writes stock.json values, keeps the file).

4. **Auto-revive watchdog** — every 30 minutes (`MIFINETUNE_WATCHDOG_SECS`,
   0 disables; host E2E uses 2) the supervisor runs an evaluate
   ("watchdog") which is always a forced reconcile: drifted profile keys
   and leftover union keys revert to the daemon's expected state — the
   parameter-lock window is now bounded at ≤ 30 minutes.

Live verification (2026-10-09, surya): first apply after the update moved
`p6.max_freq 1555200→2304000`, `cpu0/core_ctl/max_cpus 4→6`,
`vm.stat_interval 10→1`, `nr_requests 32→128`, `read_ahead 128→512`, and
healed every `policy*` floor to the profile value; the CLI restore path
returned `last_mode: hands-off` with zero node writes; the host E2E
`watchdog_and_force_apply_reconcile_every_profile_key` proves the watchdog
heals a drifted node within one cadence window.

`core_ctl` note (kernel source, surya topology **2 big + 6 little**): the
cpu0 cluster has 6 CPUs (moorea writes `min_cpus=4` with
`not_preferred = 0 0 0 0 1 1`), cpu6 cluster has 2 with core_ctl disabled —
the game/boost `min_cpus=6` writes are therefore *legal* (kernel clamps to
max=6 via `core_ctl.c store_min_cpus`), not bugs. The conservative fix is
the EXACT `6` the profiles now keep.

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
