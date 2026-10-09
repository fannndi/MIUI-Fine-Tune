# ROM Internals — reverse-engineering notes (surya / MIUI 12)

Findings from decompiling the ROM's framework + Xiaomi system apps with jadx
(`services.jar`, `framework.jar`, `MiSettings.apk`, `PowerKeeper.apk`,
`Joyose.apk`) and live device probes, 2026-10-09. The goal is to document the
MIUI-owned mechanisms MiFineTune cooperates with — and the ones it must
never fight. Everything here is read-only knowledge unless a section says the
daemon actually uses it.

Tools used (kept out of the repo): `jadx` for dex, Android `d8` for the tiny
helper dex, `mi-thermal-crypt` (user-supplied) for the encrypted thermald
configs, plain `strings`/`dexdump` where useful.

---

## 1. Display stack

### Panel modes
- Active panel on this unit: `dsi_nt36672c_huaxing_fhd_video` (POCO X3 NFC).
- Supported modes (`dumpsys display`): id 1=120, 2=90, 3=60, 4=50, 5=30 Hz.
- Device-tree dfps list (`qcom,dsi-supported-dfps-list`, big-endian u32s):
  - `nt36672c huaxing` / `nt36672c tianma`: `[120, 90, 60, 50, 30]`
  - other panels shipped on related models differ (e.g. `nt35695b` `[60,55,48]`).
- The daemon resolves the list from the active panel node at runtime and falls
  back to `[120, 90, 60, 50, 30]` (`core/src/daemon/settings.rs`, `dfps_list`).

### Who really moves the panel
`Settings.System.user_refresh_rate` alone is **cosmetic** on this ROM (verified:
key=120 while `dumpsys display` stayed on mode 3). The real chain is:

1. **MiSettings "Refresh rate"** (`com.xiaomi.misettings`,
   `RefreshRateActivity`): writes `user_refresh_rate` + `peak_refresh_rate`,
   sets the `persist.vendor.dfps.level` prop and calls
   `DisplayFeatureManager.setScreenEffect(24, hz)` (effect id 24 =
   `MODE_SCREEN_FPS_DYNAMIC_ACCOMMODATION`, per `DisplayUtils.java`).
2. **Xiaomi display-feature HAL** (`vendor.xiaomi.hardware.displayfeature@1.0`,
   `/vendor/bin/hw/vendor.xiaomi.hardware.displayfeature@1.0-service`) —
   `DisplayFeatureHal: HandleFpsSwitch: really set fps(<hz>)`, then SDM
   (`HWCDisplay::SetActiveConfig: dfps mode <idx>, the refresh rate <hz>`).
3. The HAL notifies the framework (`caseId 10035`), `ScreenEffectService`
   forwards it to SurfaceFlinger via transaction **1035**
   (`SURFACE_FLINGER_TRANSACTION_DISPLAY_FEATURE_DFPS`).

The `setScreenEffect` path only works from a root context (a shell-uid call
silently no-ops; the magisk/su context works).

**What MiFineTune uses (v0.16.1):**
- `service call SurfaceFlinger 1035 i32 <idx>` — instant panel switch
  (verified 0=120, 1=90, 2=60, 3=50, 4=30; persistent while idle).
  Quirk: the MIUI HAL helper (`setScreenEffect(24, 90)`) silently no-ops for
  90 Hz on this build ("the setting fps is the same") — the SF transaction is
  authoritative, which is why the daemon fires it first.
- `app_process` + embedded dex → `setScreenEffect(24, hz)` — keeps MIUI's HAL
  state in sync (~1 s later).
- `settings put user_refresh_rate` — keeps the MIUI UI honest.
Sleep holds 30 Hz (idx 4). Vendor dfps props are never written.

### ScreenEffectService transaction codes
Read from `com.android.server.display.ScreenEffectService` (services.jar):

| code | constant | purpose |
|------|----------|---------|
| 1035 | `..._DFPS` | dfps mode (fps) |
| 1023 | `..._SET_MODE` | color mode (also sets `persist.sys.sf.native_mode`) |
| 1036 | `..._DC_PARSE_STATE` | DC dimming parse state |
| 1100 | `..._DISPLAY_FEATURE` | WCG state |
| 1101 | `..._PCC` | picture color correction level |

### DisplayFeatureManager effect ids (framework.jar)
| id | constant | id | constant |
|----|----------|----|----------|
| 0 | SCREEN_ADAPT | 19 | SCREEN_GAME_HDR |
| 1 | SCREEN_ENHANCE | 23 | SCREEN_UNLIMITED_COLOR |
| 2 | SCREEN_STANDARD | 24 | FPS dynamic accommodation |
| 3 | SCREEN_EYECARE (paper) | 25 | DOZE_BRIGHTNESS_STATE |
| 4 | SCREEN_MONOCHROME | 26 | SCREEN_EXPERT |
| 8 | SCREEN_SUNLIGHT | 31 | SCREEN_TEXTURE_COLOR |
| 9 | SCREEN_NIGHTLIGHT | 255 | `VALUE_DISABLE_FPS_DYNAMIC_ACCOMMODATION` (no-op on this HAL build) |
| 11 | SCREEN_HIGHLIGHT | 256 | `SOCKET_FPS_SWITCH_SMART` |

Callbacks pushed up: 10000 WCG, 10035 dfps, 20000 PCC, 30000 color mode,
40000 DC parse. Client IPC = `IDisplayFeature::setFeature(0, mode, value, 255)`.

### Hidden display settings (Settings.System, MIUI-owned)
`screen_paper_mode_enabled`, `screen_paper_mode_level`,
`screen_paper_texture_level`, `screen_auto_adjust`, `screen_mode_type`,
`screen_texture_color_type`, `screen_optimize_mode`, `screen_color_level`,
`screen_monochrome_mode_enabled`, `screen_monochrome_mode`
(GLOBAL=1/LOCAL=2), `screen_monochrome_mode_white_list`, `screen_game_mode`
(1=disable eyecare, 2=enable HDR), `game_hdr_level`, `night_light_level`,
`display_color_mode`, `screen_paper_mode` (2=?), `mishow_installed`.

**Unlocked (future feature ideas):** per-app monochrome (`setScreenEffect(4,1)`),
per-app paper mode (`(3, level)`), game HDR (`(19, …)`), color modes via
transaction 1023. None are used by MiFineTune yet.

---

## 2. Performance stack (PowerKeeper, mcd, perfd)

### PowerKeeper `perfengine` (com.miui.powerkeeper)
- `PerfEngineController` registers with MIUI's `ProcessManager`
  (`miui.process.ForegroundInfo`) — richer than AOSP `UsageStats`.
- `PeGameController` loads `perf_<platform>_config` from assets
  (`perf_7X5_config` for sm7150, `perf_8X5_config`, MTK variant). Entries are
  `path#normal#boost` sysfs pairs; on foreground of a `sched_apps` package it
  writes the boost set (governor→performance, `core_ctl/min_cpus`→2, LPM rails
  off, devfreq governors→performance, ...) and restores the normal set on exit.
- `QcomBoost` calls `android.util.BoostFramework.perfLockAcquire(ms, int[])`
  / `perfLockRelease()` with hex args from `perf_lock`
  (`0x42C10000,0x1` on 7X5). QTI perf-lock, not raw sysfs.
- **Command executor = `mcd`**: commands are written to
  `/data/system/whetstone/perf_data` and applied by setting
  `mcd.extra.params = "sudebug sched <file>"` + `ctl.start = mcd_init`.
- Cloud updates land in
  `/data/data/com.miui.powerkeeper/shared_prefs/com.miui.powerkeeper.perfengine_preferences.xml`
  (currently only a ROM_VERSION key on this unit).
- `sched_apps` on 7X5 = benchmark packages (Antutu, Geekbench, ...). Daily
  apps/games only enter via cloud configs.

### perf HAL (perfd) resource ownership — audited 2026-10-10
`vendor.qti.hardware.perf@2.0-service` **is running** (`init.svc.perf-hal-2-0`),
and `machine = SDMMAGPIE`, so the `sdmmagpie` sections of
`/vendor/etc/perf/*.xml` and `/vendor/etc/powerhint.xml` apply to surya.

- **150 resources** are defined in `commonresourceconfigs.xml` (Major/Minor →
  node). The perf HAL saves the pre-existing value of every one of them in
  `/data/vendor/perfd/default_values` and restores it when a boost is
  released — that file is the authoritative "perfd-owned" list.
- Notable stock values there: `sched_migration_cost_ns=500000`,
  `sched_upmigrate=71`, `sched_downmigrate=65`,
  `sched_group_upmigrate=100`, `sched_group_downmigrate=85`,
  `sched_little_cluster_coloc_fmin_khz=740000`,
  `sched_min_task_util_for_boost=51` / `..._colocation=35`,
  `sched_load_boost` little 0 / big −6, schedutil hispeed little 1248000@90
  / big ~1324600@85, **core_ctl little min 4 / max 6**, big min 1 / max 2,
  cpusets top/foreground 0-7 + system-background/background 0-5,
  kgsl default/min 6 / max 0, LLCC bw min 4577 + DDR bw min 762,
  bw_hwmon `up_scale=250 io_percent=68 sample_ms=4 idle_mbps=1600`,
  memlat ratio_ceil/stall_floor for cpu0/cpu4, `cpu_boost input_boost_freq`
  (`0:1324800`), `vm/swap_ratio=100`.
- **19 of those overlap our catalog** (cpusets, stune top-app, up/downmigrate,
  group_up/downmigrate, coloc_fmin, many_wakeup_threshold, migration_cost,
  min_task_util_*, sync_hint_enable, kgsl default/min/max_pwrlevel). They are
  Baseline-tier by construction.
- **Empirical stomp, device-verified**: launching apps/camera changed
  `sched_group_downmigrate` 85 → 95 (perfd boost) and it stayed there — the
  daemon's event re-plan / watchdog heals it; balance does not own that node,
  game/boost do (and their 140/120 values match QTI boost conventions).
- `powerhint.xml` on this build only carries `msmsteppe`/`sdmmagpie` camera
  hints (idx 0x1331-0x1334: little `sched_load_boost` −6, hispeed load 95,
  bwmon sample_ms 20, DDR bw min 748) plus the 0x130A-0x1312 "indefinite"
  performance hints (min CPUs 2, freq floors 576/806/1248/2169 MHz).
  `perfboostsconfig.xml` boosts (0x1081/0x1082, 2 s / 400 ms) only raise
  bus-frequency floors.
- Conclusion: perfd is a **coexist** partner. Never write
  `/sys/module/msm_performance`, lpm_levels, process_reclaim, swap_ratio or
  the transient boost globals (all forbidden in the catalog guard).

### cpu_boost (touch input floor) — kernel source surya-q-oss
`drivers/cpufreq/cpu-boost.c`: an input handler hooks real touch/keypad
devices and, on every event (throttled by `MIN_INPUT_INTERVAL`), raises
`policy->min` to the per-CPU `input_boost_freq` list for `input_boost_ms`
via a `CPUFREQ_POLICY_NOTIFIER` (`cpufreq_verify_within_limits`). With the
stock seed `0:1324800 @120 ms` + `sched_boost_on_input=0`:
- every real touch floors the **whole little cluster at 1324800** for
  120 ms — which silently defeats a `powersave` (min-lock) governor;
- power key additionally uses `powerkey_input_boost_freq` (max little+big)
  for 400 ms plus a sched boost;
- synthetic `input tap/swipe` (adb/`input`) do **not** pass through
  `/dev/input` handlers, so bench results are unaffected by this.
Adopted: powersave zeroes `input_boost_freq` (disables the floor); other
profiles leave stock. Catalog key `cpu_boost.input_boost_freq`
(Baseline/Text, ALLOWED_EXACT audit in `forbidden.rs`).

### mcd (`/system/bin/mcd`, config `/system/etc/mcd_default.conf`)
MIUI root daemon; services `mcd_service` + one-shot `mcd_init`. Handles:
- `sudebug sched <file>` — apply `path#value` lines (used by PowerKeeper).
- power modes (`normal` / `idle` / `keyguard`), cgroups (`/dev/cpuset/%s`),
  memory/ZRAM tuning (`zram_size_MB` per RAM size, `global_swappiness=60`),
  `power_save` window (3000 ms, forceIdleOffPct 10),
  `/sys/class/thermal/thermal_message/boost` for camera/boost commands.

### Perf daemon (libqti-perfd)
- `ro.vendor.extension_library=libqti-perfd-client.so`, `security.perf_harden=1`;
  no `/dev/msm_performance` node observed. MiFineTune stays out of perfd's way
  (rule: `gpu.idle_timer` etc. are forbidden because perfd writes them).

### Interop risk for MiFineTune
PowerKeeper/Joyose boosts write the *same* knobs our profiles hold (governor,
`core_ctl/min_cpus`, stune). When such a boost ends it restores **stock**
values, which can stomp a profile until the next evaluate/watchdog reconcile.
Today this only happens for benchmark packages (sched_apps) and Game-Turbo
games. If we ever see complaints, options: shorter reconcile interval for the
few overlapping keys, or a "watch these paths" drift check.

---

## 3. Thermal (mi_thermald)

Configs in `/vendor/etc/thermal-*.conf` are encrypted; the scenario map is
`thermal-map.conf`:

| id | config | id | config |
|----|--------|----|--------|
| 0 | thermal-normal | 11 | thermal-class0 (missing → fallback) |
| 1 | thermal-high (missing) | 12 | thermal-camera |
| 2 | thermal-extreme (missing) | 13/16 | thermal-tgame |
| 8 | thermal-phone | 14 | thermal-youtube (missing) |
| 9 | thermal-tgame | 15 | thermal-arvr |
| 10 | thermal-nolimits | | |

Decrypted highlights (restored from CPU6 1209 MHz @50 °C in every profile):

- **normal**: big cap → **1209600** above 50.0 °C (clear 48.0); LCD throttle
  at 44/47 °C (backlight targets 819/1230); battery current targets
  201→1515 mA across 38–45 °C quiet-therm; `temp_state`=4 @60 °C;
  big-core hotplug + LCD 2048 @61 °C (`CCC_CTRL`).
- **tgame** (games): adds little cap → **1324800** above 53 °C; LCD throttles
  earlier (40/44/47).
- **phone** (calls): big → 1324800 @33 °C, then 979200 @54 °C.
- **camera**: little → 1017600 @52 °C, big → 979200 @50 °C, CPU hotplug @52 °C.
- **nolimits**: cpu7 → 1094400 @51 °C, `boost_limit`=1 @51 °C, GPU target 2.

Live state: `/data/vendor/thermal/{thermal,last_thermal}.dump`,
`decrypt.txt` (= currently active decrypted config), `thermal-global-mode`.

**Why it matters:** the balance big cap (1324800) sits exactly at Xiaomi's own
sustained values; above ~50 °C VIRTUAL-SENSOR the vendor engine clamps big to
1209600 regardless of our profile, so long sessions converge to 1.2 GHz big.
MiFineTune never edits thermal configs.

---

## 4. Game Booster / Joyose

`com.xiaomi.joyose` ships per-device cloud defaults
(`assets/<device>/default_cloud_<device>.json`). For the sm7150 family
(RedmiK30 asset): `booster_config.game_booster`:

- `migt` per-game CPU maps (`pkg;cpu:freq…;fps;intervals`):
  `com.tencent.tmgp.sgame` big cores → **1209600**,
  `com.tencent.tmgp.pubgmhd` → **1555200**, most others → 2208000.
- `support_highfps_app`: `com.tencent.tmgp.speedmobile:90`,
  `com.ztgame.bob:120`, `com.mfp.jelly.xiaomi:120`, `…anzhibdpz:120`.
- `scene_config` scene 5 "loading" → perflock `40800000_FFF#0`.
- `common_config.support_app`: 52 competitive games (Tencent/NetEase).
- `GameCenterGlobal` (`com.xiaomi.glgm`) is the game store/login component,
  not the booster backend.

This is Xiaomi's own per-app tuning data and independent validation of the
MiFineTune approach (per-app caps!). Potential future use: seed `game`
profile caps from `migt`, and treat `support_highfps_app` as a hint list for
the per-app refresh feature.

---

## 5. Charging / battery nodes

`/sys/class/power_supply/battery/` — writable set observed: `input_suspend`,
`charging_enabled`, `battery_charging_enabled`, `charge_control_limit`
(0..`charge_control_limit_max`=16), `constant_charge_current` (5.5 A) /
`_max` (6 A), `charge_term_current`, `force_recharge`, `parallel_disable`,
`fcc_stepper_enable`, `dp_dm`, `current_qnovo`, … Non-writable useful reads:
`charge_full` (5008 mAh at last check), `cycle_count` (562), `charge_counter`,
`health`, `temp`. `/vendor/bin/charge_logger` logs the charging curve; no
MIUI-side charge-limit UI exists on this build. MiFineTune uses
`input_suspend` for the charge guard and reads health from `charge_full*` /
`cycle_count`.

---

## 6. Golden rules derived from this RE

1. The panel is only moved by the HAL / SF transaction — settings keys are
   bookkeeping. Never write `persist.vendor.dfps.*`.
2. `setScreenEffect(24…)` requires root; keep it in sync with the SF switch.
3. Thermal configs are vendor-encrypted and safety-critical: read-only.
4. `mcd`, perfd, and PowerKeeper own their channels; MiFineTune uses plain
   sysfs/settings only, so their boosts and ours cannot deadlock — worst case
   is a temporary override that the next evaluate/watchdog heals.
5. New display features unlocked by this RE: per-app monochrome, paper mode,
   game HDR, color mode — all callable through the same embedded-dex channel.
