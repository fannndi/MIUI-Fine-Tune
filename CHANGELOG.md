# Changelog

## v0.11.3 — hotfix: profile CHANGE also reconciles (2026-10-09)

Live E2E on v0.11.2 caught a real hole in the full-reconcile design:
`run_once` treated "not forced" AND "already this profile id" the same,
so a plain profile id CHANGE (sleep → game) ran the apply *without* the
stock merge — the old profile's adaplers survived (core_ctl max_cpus was
still the sleep 2 while game expected the stock 6; the cpusets kept the
sleep walls). 

`run_once` now reconciles when `force` OR when the target profile id
differs from the engine's active state: keys the old profile owned but
the new one does not set revert to `stock.json`, keys the new profile
sets take their values. Periodic "same profile id" evaluations stay on
the fast path (no periodic writes).

Device proof (2026-10-09): sleep → wake → launch the mapped game →
`cpu0/core_ctl max_cpus 2→6`, `cpuset.background 0-3→0-1`,
`p6.min 1094400`, `gpu.min_pwrlevel 3`, `schedutil.pl 1` — one apply,
all values correct.

## v0.11.2 — full tryhard profile matrix (2026-10-09)

The user pushed for a full-matrix tune ("lebih tryhard"); every profile now
changes values in at least 6 keys (previously many wrote stock-identical
numbers):

- **powersave** — `p6.min_freq 652800→300000` (the big cluster can park at
  its lowest OPP when the governor is powersave) and
  `vm.vfs_cache_pressure 120→150` (more aggressive dentry/inode trim).
- **balance** — `schedutil pl 0→1` on both clusters: WALT's predicted
  load becomes a frequency floor for daily swipes (the "UI burst" the
  user asked for), still inside the harmony rules because the framework's
  launch-boost writes are swept back by the reconcile pass to stock.
- **sleep** — `p0.core_ctl.max_cpus 4→2` (4 of the 6 little cores park
  while parked), `p6.min_freq 652800→300000`,
  `p0.hispeed 1017600→768000`, `p6.hispeed 1209600→1094400`: wake bursts
  ramp to the LOWEST OPP first, a real deep-sleep pattern.
- **game** — `cpuset.background / system-background 0-5→0-1`: only two
  little cores serve background work while the top-app gobbles the rest
  (the Motivating MIUI GameTurbo move, done in our own rules).
- **boost** (5-s jank window) — full snap: `p0.hispeed 1497600→1804800`,
  `p6.hispeed 1555200→2304000` (burst straight to the OPP skyline),
  `stune.top-app.boost 10` + `prefer_idle 1` (both were missing),
  `cpuset.top-app 0-7` explicit, `gpu.max 0` (no cap during the burst)
  + `gpu.min_pwrlevel 3` (floor 380 MHz), and explicit
  `up/down_rate_limit 0/0` on both policies (instant ramp).

Invariant checks passed (host 136 tests + clippy 0 + fmt):
`core_ctl min/max pair` ✓ (sleep 2 ≤ 2; boost min=6 pre-clamps to the
kernel max_cpus), `gpu.max 0 ≤ gpu.min 3` ✓ (kgsl `max_pwrlevel ≤
min_pwrlevel`), `stune 10 ↔ 100 range` ✓, all Freq values inside the
device OPP lists (300000 / 768000 / 1094400 / 1804800 / 2304000 are all
real OPPs).

## v0.11.1 — profile identities that actually differ from stock (2026-10-09)

The user's readback: "masih tidak ada bedanya dengan stock" — audited and
CONFIRMED: the balance profile was byte-identical to the MIUI moorea
post-boot defaults in every freq/sched/core_ctl/IO/VM key (19/19 SAME),
so the daily scenario physically cannot feel different. This pass gives
every profile a distinct personality while staying inside the harmony
rules (values pre-checked against the OPP lists, core_ctl bounds, GPU
pwrlevel pair):

- **balance** becomes "stock-plus": `hispeed_freq` little 1248→1324800
  and `hispeed_load` 90→85 (swipes snap faster), `hispeed_freq` big exact
  OPP 1324600→1324800, and `core_ctl.min_cpus` 4→2 (idles two little
  cores at rest — a real battery win with an instant wake, wake cost
  ≈10 ms). Caps stay at the OPP skyline (1804800 / 2304000).
- **powersave** stays truthful saver: `p0.max` 1324800→1248000,
  `p6.max` 1555200→1324800 and `gpu.max_pwrlevel` 4→5 — a genuine clock/
  GPU cap ladder instead of the near-stock numbers it shipped.
- **sleep** gets deeper at screen-off: `p0.max` 1248000→1017600
  (at/below the own hispeed_freq — no surprise bursts while parked) and
  `p6.max` 1555200→1324800 (background sync still works, thermal cache
  much smaller).
- **boost** (jank 5 s window) fixes a real hole: it did not set
  `policy6.scaling_max_freq`, so a jank burst with a powersave base could
  not leave the powersave cap; now it writes the full 2304000.
- **game** unchanged (already distinct: 1094400 floor, 1555200 hispeed,
  1804800 little max, deadline + 1024 read-ahead, stune boost 10, pl=1).
  Rationale: those values are the tuned ceiling that survived F13 bench.

Ownership cross-check (owner-map audit still 94 entries / 0 violations):
all written values exist in the device OPP lists; `gpu.max_pwrlevel 5`
pairs with the existing `min_pwrlevel 6` (kgsl clamps 5 ≤ 6 ✓);
`core_ctl.min_cpus 2` sits inside the clamped bounds (little cluster 6
CPUs, kernel `store_min_cpus` clamps to `max_cpus`).

## v0.11.0 — full-reconcile + auto-revive watchdog + hands-off service-off (2026-10-09)

The "frequency locked and cannot drop" complaint turned out to be a
**stale-state bug**, not a tuning issue: leftover values from earlier
applies (game/boost/sleep floors, caps, IO/VM keys) survived profile
switches because the periodic fast-path only touches the current
profile's own keys.

- **Persistent `stock.json`** — union-stock map (one entry per profile
  key ever used). Seeded once (legacy snapshot fallback, else live
  device); never consumed; the engine extends it when new profile keys
  show up.
- **Full-reconcile on every forced apply** — profile switch, config/pack
  change, first apply of a session, user tap, watchdog tick: active
  profile keys take the profile value, every other union key reverts to
  stock. FreqMin/FreqMax are exact-matched in this pass
  (`readback_matches_exact`) while the periodic harmony rules stay soft.
  Worker coalescing now carries `force` sticky across a batch, so the
  startup screen-on + unlock race can no longer swallow it.
- **Auto-revive watchdog** — every `MIFINETUNE_WATCHDOG_SECS`
  (default 1800; 0 disables) the supervisor runs a forced reconcile, so
  any drift a framework/controller writes heals within 30 minutes even
  if no event happens. Host E2E proves the drift→heal loop in one cadence.
- **Hands-off service-off** — after releasing every bridge artifact the
  daemon writes nothing else: `active: null`, `last_mode: hands-off`.
  The user explicitly rejected "restore on off" as still interference.
  `miui-ft restore` stays as the explicit CLI back-to-stock repair tool.
- **RootBridge deploy content-cmp** — same-size binaries (version-bump
  builds) used to silently skip the copy; now `cmp -s` decides.
- Tested: 136 host tests (121 unit + 15 E2E), clippy 0, fmt clean;
  device proof of the reconcile healing: p6.max 1555200→2304000,
  cpu0 core_ctl max 4→6, vm.stat_interval 10→1, nr_requests 32→128,
  read_ahead 128→512 in one apply; hands-off restore left the state.json
  at `last_mode: hands-off` with zero writes.

## v0.10.0 — deep device inventory + signed battery diagnostics (2026-10-09)

Second audit loop: instead of ROM write-targets (v0.9), this pass inventoried
every readable node on the device (4 492 /proc/sys files, 433 module params,
866 CPU, 13 devfreq devices, power/bms/charger sub-supplies) and diffed them
against the catalog + forbidden + writers lists with the same classifier.

- **Guardrail expansion** (forbidden.rs): devfreq memlat/llcc/bw families,
  `gpubw`, `kgsl-busmon`, `npu`, `snoc_cnoc_keepalive`, `ufshc`, GPU devfreq
  (`5000000.qcom,kgsl-3d0`), `mmc0` devfreq, vidc video devices; cpuidle
  state gating (nothing writes it, blocking deep idle burns battery); the
  `sched_coloc_busy_hyst*`/`coloc_downmigrate` nodes the ROM writes on other
  SoCs but which are absent on this kernel; charger/parallel/USB power paths
  (`main`, `dc`, `usb`, `bms`, `bq2597x-standalone`, `pc_port`) — micharge's
  surface, ours stays under `/battery`.
- **Signed battery diagnostics** (read-only, all 0444/0400 root): `EnvSnapshot`
  grew `battery_current_ua` (negative = **charging**, positive = discharge —
  sign verified with a controlled `input_suspend` bypass test: +600 mA
  discharge during suspend, negatives on restore), `battery_voltage_uv`,
  `little/big_freq_mhz` (cpuinfo_cur_freq, kHz), `gpu_freq_mhz`
  (kgsl gpuclk is Hz) and `storage_written_kb` (f2fs lifetime counter,
  53 GB total). Diagnostics card gained "Battery I/V", "CPU L/B MHz",
  "GPU MHz", "Storage written"; doctor gained `env_power` + `env_freq`.
- **`sched_boost` stays forbidden, observation recorded**: MIUI/perfd writes
  1 transiently during app-launch bursts and clears it (live-sampled during
  the game E2E): the catalog never writes it, BY the harmony rule — the
  forbidden entry now has live evidence.
- **Rejected with evidence**: `sched_cstate_aware` (kernel default 1, no
  writer, placement trade-only — not profile-proof), `cpuidle stateN/disable`
  (battery-harmful, no consumer), `scaling_boost_frequencies` (0444), PSI
  (`/proc/pressure/*` absent on kernel 4.14), `compaction_proactiveness`
  (absent), `sched_coloc_busy_hyst*` (absent on this kernel).
- Tests: 119 unit + 13 E2E, clippy 0, fmt clean; owner-map audit 94 entries /
  0 violations with the expanded guardrail.

## v0.9.0 — ROM compatibility audit + Predictive Load + panel FPS (2026-10-09)

A systematic audit of every configuration file across all ROM partitions
(359 XML, 231 prop, 179 RC, 62 conf, 51 JSON, 38 sh), automated for reuse:

- **New tool `tools/rom-write-audit.sh`**: extracts every `write`/`echo`
  target from a ROM tree (546 on V12.0.9), classifies each against the
  catalog/runtime-writers/forbidden lists, and with `--device` probes
  existence as root (205 exist — plain uid loses 108 permission-gated
  `/proc/sys` nodes) plus a **live-vs-boot value diff** that flags unknown
  runtime writers (result: 0 after analysis).
- **Catalog 91 → 94**: `policy0/policy6 schedutil pl` (Baseline — perf XML
  declares the resource but no boost uses it; the executed moorea arm never
  writes it; live watch stayed 0 during a game boost; write/readback ✓) and
  `kernel.sched_conservative_pl` (Baseline — written only by the lito/atoll
  arms; our arm leaves the kernel default 0).
- **Predictive Load profiles** (semantics from
  `kernel/sched/cpufreq_schedutil.c`): `pl=1` floors util by WALT's
  *predicted* load (early ramp), `conservative_pl=1` damps it. game + boost
  get `pl=1` (game damped, boost undamped); powersave/balance/sleep stay
  reactive `pl=0`.
- **Panel FPS telemetry** (read-only): the DRM `measured_fps` node feeds
  `EnvSnapshot.screen_fps` → `env` events → Diagnostics "Panel FPS" row.
  Device-verified: idle 75.1 → swipes → 1.5 fps.
- Audit rejections recorded with evidence: touch nodes are 0444 read-only;
  `dsi_display_hbm`/`cabc` are property-triggered MIUI controllers;
  `big_cluster_min_freq_adjust` and most `sched_*` targets don't exist on
  this SoC; `charging_enabled` is Free but unused (not cataloged); two
  earlier suspicions corrected (rmem_max has two boot writers, sched_load_boost
  differs by branch arm).
- Tests: 119 unit + 13 host E2E, clippy 0, fmt clean; owner-map audit 94
  entries, 0 violations.

## v0.8.0 — Apps Profile (software layer) + F9 removal (2026-10-08)

The two profile layers are now explicit: **Device Profile** is the hardware
catalog (unchanged), **Apps Profile** is a per-app software layer with an
explicit Save button. Nothing MIUI controls is overwritten.

- **F9 removed**: MiFineTune no longer manages the refresh rate at all
  (MIUI's own toggle owns it). The refresh-follow bridge, config flag,
  Settings toggle and tests are gone; a one-time migration gives a held
  refresh value back if an old `holds.json` still carries it.
- **Apps Profile v2** (`app_profiles`): per-app Device-Profile mapping +
  bypass charging + DND. `app_map` stays as a backward-compatible mirror and
  legacy mappings survive every write.
- **Bypass charging** (opt-in): `input_suspend` (Baseline, ALLOWED_EXACT #2)
  suspends charger input while the app is in front — device runs on battery,
  charging heat disappears. Engages above `bypass_floor_pct` (Settings
  slider 15..50, default 30) + 5 % hysteresis, releases at the floor, on app
  exit, Service OFF and crash recovery. Device E2E: `bypass charging ON
  (com.YoStarEN.AzurLane, 100%)` → status `Discharging` → leave → `bypass
  charging OFF (node 0)` → `Charging`.
- **DND per-app** (opt-in): the daemon decides, the app executes through the
  official `NotificationManager.setInterruptionFilter` (one-time "Do Not
  Disturb access" grant; `zen_mode` is never written directly). Device E2E:
  `dnd applied: total` (zen 1→2) → leave → `DND released` (zen 2→1, the
  user's own state).
- **Apps Profile UI**: list with All/Games/Configured filters and summary
  chips (`Game · Bypass · DND`), a detail screen with draft state, an
  explicit **Save app profile** button, discard confirmation and reset.
- **Settings**: new "Charging" card (charge limit + bypass floor slider).
  Diagnostics holds row shows bypass/DND.
- **Dropped: per-app HWUI renderer** — the app-spawned daemon runs in
  `untrusted_app`, where SELinux denies `debug_prop` writes; no workaround
  is acceptable under the harmony rules.
- Tests: 119 unit + 13 host E2E (bypass/DND E2E over fake sysfs/settings),
  clippy 0, fmt clean; owner-map audit passes (91 entries).

## v0.7.0 — telemetry, guards, maintenance & FAS-lite (2026-10-08)

Read-only telemetry, transition history, an in-app Diagnostics screen and
the first env-aware guards — the foundation for everything after.

- **Env sampler** (`engine/env.rs` + `daemon/env.rs`): 30 s thread reading
  battery %/status/temp, the best CPU and GPU thermal zones (resolved by
  `type` at startup), and Adreno GPU busy; forwarded as `env` events on
  change. `MIFINETUNE_SYSFS_ROOT` points host tests at a fake tree;
  `MIFINETUNE_ENV_SAMPLE_MS` overrides the cadence.
- **Adaptive guards** (daemon + arbiter):
  - battery guard: at/below `battery_floor_pct` (default 20) while not
    charging → Power Save, beating mapping/saver/multi-window; sleep and
    keyguard still win; release is immediate once charged;
  - thermal guard: Game (mapped or base) steps down to Balance at
    `thermal_ceiling_c` (default 75 °C), releases 5 °C lower (hysteresis);
    a missing sensor keeps the last state;
  - guard-verdict flips trigger `evaluate("env")` immediately.
- **Refresh-rate follow** (bridge target #3, v0.7): mapped game → 120 Hz,
  powersave-mapped app → 60 Hz via `Settings.System user_refresh_rate`; the
  user's captured value returns on release and the vendor DFPS prop is never
  touched (device-verified that the framework honors the setting live).
- **Catalog expansion (v0.7, 84 → 87)**: `io.read_ahead_kb` (Free:
  boot scripts write only dm-*/mmcblk*; the sda/userdata 512 is the Xiaomi
  kernel patch `VM_MAX_READAHEAD=512`; profiles 128/512/1024) and the
  `cpu0/cpu6 l3-lat` devfreq `min_freq` floors (Baseline: written by the
  SKU blocks in post_boot; game floor 940800000 = the SA6150 post_boot
  value, others stock 300000000). Audited against the pulled
  V12.0.9 ROM + kernel source; kgsl micro knobs stay out (the perf HAL
  runtime-writes the safe ones — force_clk_on/no_nap/idle_timer — and the
  rest are hang-recovery/firmware semantics).
- **Jank boost (experimental, opt-in)**: a third logcat watcher parses
  `Choreographer: Skipped N frames!` (freshness-checked); 10+ skipped frames
  raise a hidden `boost` profile (responsive CPU floors) for ~5 s, then the
  supervisor returns to the normal decision. Rate-limited by a 30 s
  cooldown; returns/extends while the window is active. Host E2E drives the
  full round trip over the fake logcat.
- **Charge guard (opt-in)**: pause charging at a configurable limit
  (default 80 %, release 5 % lower), using the ROM's own user-facing
  `battery_charging_enabled` switch (init.target.rc chmod 0777 — the single
  `power_supply` path in ALLOWED_EXACT, Baseline). The hold captures the
  stock value and returns it on release/service-off; the guard is fed by
  every env sample so it works overnight with the screen off. Device E2E:
  node 1 → 0 → 1, `charge paused at 100% (limit 80)` / `charge resumed`,
  release verified through Service OFF (restore retry by design).
- **Storage maintenance (opt-in)**: weekly bounded f2fs GC while charging +
  screen off, reusing the ROM's own `checkpoint_gc` pattern (short
  `gc_urgent_sleep_time`, `gc_urgent=1`, poll `dirty_segments` to ≤100,
  restore, `sync`; capped at 10 min). Two Baseline catalog nodes; runs on a
  daemon thread, result persisted in `maintenance.json` and logged.
  Device run: `gc 3521 -> 30 dirty segments in 70s`, nodes restored.
- **Pack-update reconciliation**: profiles.json is mtime-watched; the first
  apply of every daemon run and every config/pack hint forces a re-plan, so
  new catalog keys reach the device without waiting for an app switch (the
  old `active == profile` skip could leave them stale). `snapshot.json`
  gains new keys automatically on the first apply (first-write-wins), so
  restore stays byte-exact.
- **Transition history** (`daemon/stats.rs`): one entry per real profile
  switch (from/to/reason/battery/temp) persisted atomically to
  `stats.json` (500-entry cap, survives daemon restarts), served via `stats`.
- **IPC**: new commands `diag` (daemon health: pid, uptime, watchers, holds,
  config, env, history size) and `stats`; new events `env`/`diag`/`stats`.
- **App**: `DiagnosticsScreen` (daemon health, live environment, 24 h
  time-in-profile, transition timeline, relayed daemon log with a 200-line
  in-memory ring); Diagnostics row on Home with a live battery/temp/GPU
  summary; `GuardsCard` in Settings (toggles + sliders, values commit on
  release); Quick Settings tile (service toggle + active profile subtitle);
  suggested game mappings in the Apps screen; JSON config export/import via
  the system file picker (validated + clamped, unknown keys ignored).
- **Doctor v2**: env telemetry checks, future-knob surface probe (f2fs /
  devfreq / kgsl / read-head), refresh-rate key, logcat `-v epoch` format.
- **Tests**: 114 unit + 13 host E2E across three binaries (protocol/decision,
  watcher streams incl. jank boost, bridge hold/restore incl. charge
  hysteresis; fake-sysfs env/guard E2E; profile pack update E2E; maintenance
  trigger E2E).
- **CI** (`.github/workflows/ci.yml`): Rust job (fmt check, clippy with
  `-D warnings`, all tests) and an Android job (NDK arm64 core build +
  `assembleDebug`); the identical coverage runs locally with `cargo test`.
- **Host simulation**: `MIFINETUNE_SETTINGS_BIN` joins the existing
  logcat/sysfs overrides — the bridge hold/restore cycle is now proven
  host-side through the real `settings` plumbing. The daemon creates its
  state dir at startup (holds/stats are writable from the first tick), and
  bridge mode changes are emitted as `bridge` events (in-app timeline).
- **Device E2E (POCO X3)**: doctor v2 all green; diagnostics screen shows
  live battery/thermal/GPU, uptime, watchers, holds; `stats.json` records
  `sleep -> game -> powersave` with battery/temp and survives restarts;
  fake-sysfs CLI runs on device prove `low battery` → Power Save and
  `thermal` → Balance; guards config roundtrip through the UI verified;
  refresh follow verified live (game → 120 Hz, home → restore the user's
  60 Hz, final value restored); backup export/import round-trip through the
  system picker (import reloaded the daemon and applied the new base);
  QS tile declared with the system permission; 24 h dashboard shows
  `Power Save 41m · Sleep 2m · Game 1m`; catalog additions verified live
  (game read_ahead=1024 / l3 min=940800000, sleep+powersave 128/300000000);
  the pack-update path verified end to end (stale pack + stale values →
  daemon start → `forced re-plan` → values reconciled, no switch needed).

## v0.6.0 — full-Rust daemon (2026-10-08)

Architecture overhaul: every decision and every write moved into a Rust
daemon; Kotlin is now a thin UI + lifecycle client.

- **Daemon** (`miui-ft serve`): stdio JSON-lines, spawned once via `su`;
  stdin EOF is the shutdown signal. In-process engine (no `su` round-trips
  per decision), dedicated threads for applies, watchers and MIUI bridge IO.
- **Arbiter** ported to Rust with the full decision table (incl. dynamic
  gate, multi-window, saver attribution); JVM tests moved to `cargo test`.
- **Watchers** in Rust: `logcat -v epoch` streams for foreground resume and
  the GameBooster multi-window line; wake/unlock seed via one-shot peek;
  freshness is epoch arithmetic (the old 1970 parse bug class is gone).
- **MIUI bridge** in Rust: hold/restore state machine, perf mirror + saver
  follow via the `settings` CLI, game-mode checker; holds persisted to
  `holds.json` (crash-safe); one Mutex serializes state+IO.
- **Config** moved from SharedPreferences to app-owned `config.json`
  (atomic writes; one-time migration; prefs kept as downgrade fallback).
- **App**: service is an FGS shell (notification, broadcasts, daemon
  restart); apply/restore route through the daemon when connected.
- **Engine** split into small modules (`engine/` tree, all files ≤ ~300
  lines) with a module index; no behavior change.
- **Tests**: 84 Rust unit tests + 4 host E2E tests (daemon protocol ×3,
  watcher pipeline with a fake logcat).
- **Device E2E (2026-10-08, POCO X3)**: pipe spawn + hello, config
  migration, sleep timer + wake seed, live foreground events, mapped
  game -> game + perf mirror, HOME -> base, Dynamic OFF/ON, Service
  OFF restore (27 keys verified) + daemon bye, Service ON restart,
  force-stop recovery, split-screen -> balance + back, notification
  text, MW parser vs real GameBoosterService lines.
- **Docs**: README, AGENTS, ARCHITECTURE, IPC-PROTOCOL, ROM-HARMONY — all
  English.

## v0.5.0 — Dynamic Profile (2026-10-08)

- Renamed `automation` → `DynamicProfile` (package, classes, prefs file
  with migration).
- New **Dynamic Profile** switch: OFF = the universal base always wins,
  mappings ignored; MIUI bridge syncs are gated on it.
- Service row icon → PowerSettingsNew; Dynamic row added.
- Settle window 700 → 400 ms (measured); latency instrumentation
  (`apply <id>: done in <ms> (settle <ms>)`).

## v0.4.2 — MIUI bridge (2026-10-08)

- MIUI battery-saver follow (live `low_power` write + hold/restore).
- Performance mirror via `Settings.System power_mode` (the real property
  is SELinux-locked; the mirror is the only writable surface).
- Game-mode checker notification; multi-window → Balance rule
  (`GameBoosterService` signal).
- Watcher hardening: dual resume tags, Calendar-based freshness, MW buffer
  replay as seed.

## v0.3.0 — service + drift guard

- Foreground service with event-driven switching (`logcat -b events`),
  sleep profile, base/mapped model, apps mapping UI.
- Drift guard (periodic re-apply; unchanged keys skipped in the engine).

## v0.2.0 — profiles + detail

- Profile cards, detail dialog, plan/apply/verify/restore CLI.

## v0.1.0 — engine

- Rust engine: catalog, probe, snapshot, guarded writes, read-back verify.
