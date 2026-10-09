# AGENTS.md — MiFineTune

Guide for AI agents and humans working on this repository.
Device: **POCO X3 NFC, surya / sm6150 / MIUI 12 / Android 10**, root APatch.
Read [docs/ROM-HARMONY.md](docs/ROM-HARMONY.md) before touching the engine or
catalog; read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before touching the
daemon or the app↔daemon boundary.

## Architecture in one paragraph

The Rust daemon (`core/src/daemon/`, started as `miui-ft serve`) owns every
decision and every write. The Kotlin app is a thin client: Compose UI,
foreground service + notification, `su` process plumbing, and Android-only
signals (screen/keyguard/ultra-saver broadcasts). They talk JSON-lines over
stdio; `config.json` (app-owned, in `filesDir`) is the only shared file for
user intent. The engine (`core/src/engine/`) is the only code that writes
parameters, always through the catalog guard.

## Profile model — Device vs Apps (hard rule)

Two layers, two scopes. Keep them separate; never overwrite what MIUI controls.

| | Device Profile (hardware) | Apps Profile (software) |
|---|---|---|
| Scope | device-wide, always on | per-app, only while that app is foreground |
| Content | the 91-node catalog (CPU gov/freq, core_ctl, sched/cpuset/stune, GPU, IO, VM, net) + the MIUI power mirror | Device-Profile mapping + the audited software surfaces |
| Writes | engine plan from `profiles.json` | bridge holds: capture -> apply -> restore |
| UI | Home "Device Profile" cards | Apps Profile list -> detail screen |

Rules:
1. A parameter belongs to exactly one layer; the catalog is hardware-only.
2. MIUI-owned parameters are never written: refresh rate
   (`user_refresh_rate`/dfps), MIUI power modes (`persist.sys.aries.*`),
   GameTurbo (`gb_boosting`/`vtb_boosting`/`screen_game_mode`), thermal, perf
   locks, LMK/zram, JEITA/step-charging, direct `zen_mode` writes.
3. Per-app features use only audited user-facing surfaces: `input_suspend`
   (bypass charging) and the official DND API (`dnd` event -> the app calls
   `NotificationManager.setInterruptionFilter`). Every effect is restored on
   app exit, Service OFF and daemon recovery (`holds.json`).
4. Per-app gates: `dynamic && service enabled`; an absent field leaves MIUI
   untouched.
5. Refresh rate is MIUI's; MiFineTune does not manage it (F9 removed). The
   HWUI renderer prop was dropped: the app-spawned daemon runs in
   `untrusted_app`, where SELinux denies `debug_prop` writes.

## File map (one line per file — keep this current)

### Rust (`core/`)

| Path | Responsibility |
|---|---|
| `src/lib.rs` | module index (`daemon`, `engine`, `DEFAULT_PROFILES_JSON`) |
| `src/main.rs` | CLI entry: probe/profiles/plan/apply/restore/verify/status/serve/doctor/catalog |
| `profiles.json` | profile pack (embedded via `include_str!`, mirrored to assets) |
| `src/engine/mod.rs` | engine module index + test fixtures hookup |
| `src/engine/catalog/mod.rs` | tier/kind/entry types, `find`, `guard_path` (forbidden-path guard) |
| `src/engine/catalog/entries.rs` | the 94-node registry table (data only) |
| `src/engine/catalog/forbidden.rs` | framework-owned path prefixes + exact keys (never written) |
| `src/engine/env.rs` | read-only telemetry sampler (battery / thermal / GPU busy) |
| `src/engine/probe.rs` | read-only device capture with `MIFINETUNE_SYSFS_ROOT` IO root (`read`/`write`/`exists` all rooted; double-prefix safe) |
| `src/engine/profile.rs` | `profiles.json` model + parser |
| `src/engine/plan.rs` | `build_plan`: statuses, kernel-invariant pair checks, write ordering |
| `src/engine/validate.rs` | per-kind value validation (OPP clamping, core_ctl bounds) |
| `src/engine/readback.rs` | read-back comparison rules + snapshot normalization |
| `src/engine/apply/mod.rs` | apply/restore/verify module index + shared report types |
| `src/engine/apply/store.rs` | state dir: atomic JSON, snapshot capture, persistent `stock.json` (union-stock seed/extend), profile loading |
| `src/engine/apply/write.rs` | guarded writes, kernel-safe ordering, pass-2 re-plan |
| `src/engine/apply/restore.rs` | CLI back-to-stock (writes `stock.json`; the daemon never calls this) |
| `src/engine/apply/verify.rs` | drift detection + verified read-back helper |
| `src/engine/doctor.rs` | `doctor` environment self-check (JSON) |
| `src/engine/testutil.rs` | shared engine test fixtures (`cfg(test)`) |
| `src/daemon/mod.rs` | daemon wiring: constants, state type, threads, main loop |
| `src/daemon/runtime.rs` | state emission + applied/restored event handling |
| `src/daemon/commands.rs` | command dispatch (IPC + watcher messages) |
| `src/daemon/evaluate.rs` | decision point + 3 s supervisor timer |
| `src/daemon/proto.rs` | IPC wire format (Command/Event) + stdout Publisher |
| `src/daemon/config.rs` | `config.json` cache (app writes; daemon reloads) |
| `src/daemon/arbiter.rs` | pure decision table (ported from Kotlin; all cases tested) |
| `src/daemon/worker.rs` + `worker/tests.rs` | coalescing apply worker: settle/supersede/retry/restore-cancel |
| `src/daemon/engine_driver.rs` | engine adapter for the worker: `apply(id, reconcile)` + **hands-off restore** (release holds only) |
| `src/daemon/watcher.rs` | logcat watchers (`-v epoch`): foreground + multi-window + jank + peek |
| `src/daemon/env.rs` | env sampler thread (read-only telemetry into the loop) |
| `src/daemon/maintenance.rs` | opt-in weekly f2fs GC window (charging + idle, bounded) |
| `src/daemon/stats.rs` | transition history (bounded, persisted `stats.json`) |
| `src/daemon/bridge/mod.rs` | Bridge: shared state + one Mutex, recover/release/persist |
| `src/daemon/bridge/holds.rs` | PowerMode + hold/restore state machine + holds.json |
| `src/daemon/bridge/sync.rs` | SyncCtx + pure gates + settings-CLI sync IO |
| `src/daemon/bridge/charge.rs` | charge guard (`battery_charging_enabled`, opt-in limit) |
| `src/daemon/bridge/bypass.rs` | per-app bypass charging (`input_suspend`, floor + hysteresis) |
| `src/daemon/bridge/dnd.rs` | per-app DND decision (the app executes the official API) |
| `src/daemon/settings.rs` | `settings` CLI read/write helpers (saver, power_mode) |
| `tests/daemon_smoke.rs` | end-to-end protocol tests: full decision path, EOF exit |
| `tests/daemon_watchers.rs` | watcher E2E over a fake logcat script |
| `tests/daemon_bridge.rs` | bridge E2E over a fake settings binary (hold/restore) |
| `tests/common/mod.rs` | shared spawn/drive harness for integration tests |

### Kotlin (`app/src/main/java/com/mifinetune/`)

| Path | Responsibility |
|---|---|
| `MainActivity.kt` | single activity, Compose host |
| `core/RootBridge.kt` | `su` exec + binary/profile deploy (assets → device) |
| `core/FtClient.kt` | typed CLI client (JSON parsing) for service-off paths |
| `core/Models.kt` | CLI JSON shapes (status/plan/report/profiles) |
| `core/Tuner.kt` | serialized CLI access + manual drift guard (service-off only) |
| `dynamic/DaemonClient.kt` | daemon spawn, JSON-lines framing, stderr→logcat relay, `DaemonLink` |
| `dynamic/DynamicProfileService.kt` | FGS lifecycle; forwards screen/keyguard/ultra; maps events to state |
| `dynamic/DaemonNotifications.kt` | notification channels + builders (service notification, GM warning) |
| `dynamic/DiagnosticsModels.kt` | diagnostics shapes (env / diag / stats) + JSON parsing |
| `dynamic/ServiceTile.kt` | Quick Settings tile: service toggle + active profile subtitle |
| `dynamic/DynamicProfileConfig.kt` | `config.json` writer (atomic), prefs migration, daemon hints |
| `dynamic/AppProfileEntry.kt` | Apps Profile entry model (mapping + bypass + DND) |
| `dynamic/DndController.kt` | the only platform-API executor: DND access + interruption filter |
| `dynamic/DynamicProfileState.kt` | process-wide StateFlows mirrored from daemon events (incl. env/diag/stats/logs) |
| `dynamic/DeviceContext.kt` | PowerManager/KeyguardManager reader only |
| `dynamic/DynamicProfileBootReceiver.kt` | best-effort service start on boot |
| `ui/HomeScreen.kt` | screen scaffold + navigation + service-off confirm |
| `ui/HomeRows.kt` | profile cards, Apps entry, Service + Dynamic Profile rows |
| `ui/HomeDialogs.kt` | report / profile detail / locked-keys dialogs |
| `ui/HomeViewModel.kt` + `HomeUiState.kt` | Home controller + UI state shapes |
| `ui/DiagnosticsScreen.kt` | diagnostics screen (daemon health, env, history, log) |
| `ui/StatsSummary.kt` | 24 h time-in-profile summary from the transition history |
| `ui/SettingsBackup.kt` | config export/import through the system file picker |
| `ui/SettingsGuards.kt` | adaptive guards card (battery floor / thermal ceiling) |
| `ui/SettingsCharging.kt` | charging card (charge limit + per-app bypass floor) |
| `ui/AppProfileDetailScreen.kt` | Apps Profile detail (draft + explicit Save) |
| `ui/DynamicProfileViewModel.kt` | apps list + settings toggles controller |
| `ui/AppsProfileScreen.kt` / `SettingsScreen.kt` / `UiBits.kt` / `ProfileLabels.kt` / `theme/` | Compose UI |

### Other

| Path | Content |
|---|---|
| `docs/ARCHITECTURE.md` | layers, daemon thread model, data flow, precision policy |
| `docs/IPC-PROTOCOL.md` | stdio protocol schema + lifecycle |
| `docs/ROM-HARMONY.md` | node ownership map, kernel invariants, audit findings |
| `tools/bench.sh` | on-device benchmark harness (CLI-based) |
| `data/adb/mifinetune/stock.json` | persistent union-stock baseline (dev-seeded once; engine extends, never consumes) |
| `tools/owner-map-audit.sh` | catalog vs ROM audit (boot + runtime writers) |
| `tools/rom-write-audit.sh` | ROM write-target extractor + classifier (`--device`: root probe + live-vs-boot value diff) |
| `tools/display-off-diff.sh` | empirical display-off behavior test |
| `tools/perf-hal-runtime-writers.txt` | runtime writer evidence list |
| `.github/workflows/ci.yml` | CI: fmt + clippy + full host tests, then Android build |

## Host simulation (fixture overrides)

Integration tests and simulations replace device IO with fixtures via env
vars — the daemon code paths are the real ones:

| Env var | Replaces | Used by |
|---|---|---|
| `MIFINETUNE_LOGCAT_BIN` | `/system/bin/logcat` | watcher E2E (fake streams) |
| `MIFINETUNE_SETTINGS_BIN` | `/system/bin/settings` | bridge E2E (fake hold/restore) |
| `MIFINETUNE_SYSFS_ROOT` | `/sys` | env sampler + guard E2E (fake tree) |
| `MIFINETUNE_ENV_SAMPLE_MS` | 30 s sample cadence | guard reaction tests |
| `MIFINETUNE_WATCHDOG_SECS` | 1800 s auto-revive cadence (0 = off) | reconcile watchdog E2E |

`cargo test` covers: engine units, daemon protocol/decision E2E, watcher
streams, bridge hold/restore, env guards and the doctor — no device needed.

## Hard rules (never break these)

1. **Harmony guard** — the engine writes only Free/Baseline catalog nodes;
   framework-owned paths are rejected by `guard_path` on every write. Never
   add a write path that bypasses the engine guard. Never touch SELinux
   (`/sys/fs/selinux`, `setenforce`, `restorecon`), never attempt `setprop`
   for `persist.sys.aries.power_profile` (SELinux rejects it on this device).
   The app-spawned daemon runs in `untrusted_app`, where every `debug_prop`
   write is denied too — the HWUI renderer feature was dropped for this
   reason; do not reintroduce prop writes.
2. **Single writer** — all parameter writes go through the daemon (or the CLI
   when the service is off). The app never writes sysfs/proc directly.
3. **Ownership evidence** — adding a catalog entry requires tier evidence
   (post_boot + perf XML + runtime writers + device); see ROM-HARMONY.md.
4. **Coexist, never fight** — nodes the framework rewrites at runtime are at
   least Baseline; nodes rewritten continuously (e.g. `net.tcp_rmem/wmem`)
   stay out of profiles entirely (there is an automated test for this).
5. **Restore = byte-exact stock, or hands-off** — the daemon never calls
   `apply::restore` (service OFF = release holds + `active=null`, zero
   node writes, per the v0.11 user ruling); the CLI `miui-ft restore`
   writes the stock.json values verbatim and must keep them byte-exact.
6. **Kernel invariants** — keep `ordered_pairs_with` and the pass-2 re-plan;
   they encode kernel rejection rules (see ROM-HARMONY.md invariants table).
7. **Asset sync** — `assembleDebug` runs `syncCore` which copies
   `core/profiles.json` + the release binary into `app/src/main/assets/`.
   If you change the build flow, keep both copies in lockstep (stale assets
   silently revert the device binary — this happened twice in history).
8. **Device testing is mandatory** for engine/daemon changes; unit tests are
   not sufficient (kernel/thermal/MIUI behavior cannot be mocked).

## Precision policy (why the code looks the way it does)

- Durations use `Instant` (monotonic); wall-clock is only for log stamps.
- Logcat parsing uses `-v epoch` (epoch arithmetic); no timezone/year parsing.
- IPC is strict JSON-lines: oversized/invalid lines are logged and skipped,
  never fatal; `stdin` EOF is the shutdown signal.
- `config.json`, `holds.json`, `state.json`, `snapshot.json` are written
  atomically (tmp + rename).
- Read-back verify is a hard gate; "already active" skips only via engine
  state, never via assumption.

## Build & test

```bash
# Rust (host): 119 unit tests + 13 host E2E tests
cd core && cargo test

# cross-build arm64 + assets (syncCore also runs automatically in Gradle)
ANDROID_HOME=$HOME/Android/Sdk cargo ndk -t arm64-v8a build --release
cp target/aarch64-linux-android/release/miui-ft ../app/src/main/assets/miui-ft

# app
cd .. && ./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

## Device E2E checklist (run after engine/daemon changes)

```bash
B=/data/local/tmp/mifinetune/miui-ft
export PATH=$PATH:$HOME/Android/Sdk/platform-tools

# daemon protocol on device (app must be running with service ON)
adb shell "su -c '$B serve --state-dir /data/adb/mifinetune --config <path>'" < <(echo '{"cmd":"hello"}')

# CLI smoke
adb shell "su -c '$B plan balance'"
adb shell "su -c '$B apply balance'"
adb shell "su -c '$B verify balance'"
adb shell "su -c '$B restore'"

# app flow
adb logcat -s MiFineTune:*   # daemon stderr is relayed here
# - Dynamic ON: open a mapped game -> 'evaluate(event): -> game (app)'
# - leave game -> base; screen off -> sleep after ~10 s; wake -> base/game
# - Dynamic OFF: mapped game stays base; no 'bridge: MIUI ...' lines
# - Service OFF: 'restore: ok=true' + 'stdin closed, exiting'
# - split screen: 'bridge: multi-window ON (...)' -> balance

# diagnostics (Phase 7)
adb shell "su -c '$B doctor --state-dir /data/adb/mifinetune'"  # env_* checks pass
# in-app: Home -> Diagnostics shows live env, transitions and the daemon log
adb shell "su -c 'cat /data/adb/mifinetune/stats.json'"         # one entry per switch

# host-side telemetry simulation (no device needed)
cd core && MIFINETUNE_SYSFS_ROOT=/path/to/fake-root cargo test --test daemon_smoke

# env guards on device (fake sysfs; real nodes untouched by the guard test):
#   battery 10%  -> 'applied ... reason=low battery'
#   CPU 80 °C    -> 'applied ... reason=thermal'

# v0.7 feature checks (all verified 2026-10-08):
# - Apps Profile (v0.8): a configured app in front ->
#   'bridge: bypass charging ON (pkg, N%)' + node input_suspend=1;
#   'bridge: DND total (pkg)' + app log 'dnd applied: total' (needs the
#   one-time DND access grant); leave -> 'bypass charging OFF' + 'DND released'
# - storage maintenance (opt-in, charging + screen off):
#   'maintenance: done (gc 3521 -> 30 dirty segments in 70s)';
#   /data/adb/mifinetune/maintenance.json records the run
# - charge limit (opt-in): toggle -> 'bridge: charge paused at N% (limit X)'
#   (node 1 -> 0); toggle off / Service OFF -> 'bridge: charge resumed'
# - jank boost (opt-in): load + heavy swipes -> 'jank: N frames — boosting'
#   -> 'apply boost: done' -> 'boost window over' -> normal profile
# - v0.9: Diagnostics shows 'Panel FPS' (live, read-only measured_fps);
#   mapped game -> profile writes policy0/6 schedutil pl=1 + conservative=1
#   (read back via 'cat .../schedutil/pl'); tooling:
#   tools/rom-write-audit.sh <rom> --device -> 0 value-diff unknown writers
# - v0.10: Diagnostics also shows Battery I/V (sign: minus = charging,
#   verified via controlled input_suspend bypass test), CPU L/B MHz,
#   GPU MHz, Storage written; forbidden.rs owns devfreq memlat/bw families,
#   cpuidle gating and charger-HW paths (main/dc/usb/bms) — our charge
#   surface stays strictly under /battery (no catalog writes sched_boost,
#   MIUI/perfd owns that transiently)
# - v0.11: first apply + every forced apply reconciles the FULL union
#   (profile keys -> profile value; every other union key -> stock.json).
#   Service OFF writes nothing ("last_mode":"hands-off", active:null).
#   Watchdog reconcile every 30 min (MIFINETUNE_WATCHDOG_SECS); device
#   proof: p6.max 1555200->2304000, cpu0.core_ctl.max_cpus 4->6,
#   vm.stat_interval 10->1, nr_requests 32->128 healed in one apply.
#   RootBridge deploy now content-cmp (a 0.10->0.11 same-size binary used
#   to silently skip the copy)
# - diagnostics: Home -> Diagnostics = env + 24 h + transitions + daemon log
# - backup: Settings -> Backup export/import through the system picker
# - v0.13.0: battery care v2 + auto-revive visibility. EnvSnapshot gains
#   charge_full_mah + cycle_count (charge_full_design is broken/negative on
#   surya -> spec 5160 mAh used for the wear estimate). config.charge_once
#   skips the charge limit until the next unplug; the daemon emits
#   charge_once_done and the APP clears the flag (config.json stays
#   app-owned). Watchdog applies are tagged (worker Job.watchdog) and a
#   successful heal increments stats.json heals_total/heals_last_keys/
#   heals_last_t; Diagnostics shows the Auto-revive card. Device-verified
#   2026-10-09: charge-once skip -> pause -> resume cycle + health fields
#   live (5008 mAh / 562 cycles); host E2E for both paths.
# - v0.12.0: round-2 profile matrix: core_ctl busy_up/down/offline_delay
#   thresholds, io.cfq battery batching (fifo_expire_async/slice_async/
#   low_latency), io.rq_affinity, vm.watermark_scale_factor, WALT group
#   migrate pair (down<=up, validated + write-ordered), coloc_fmin
#   (range 0..=2M), walt_rotate, net.tcp_slow_start_after_idle,
#   gpu.default_pwrlevel. Device proof 2026-10-09: balance busy 60->55 /
#   40->35 / delay 100->120, rq 1->2, watermark 1->5, coloc 740000->
#   940800, slowstart 1->0; game group 100/85->140/120, walt_rotate 0,
#   default_pwrlevel 4, busy 35/15/400, watermark 10; sleep busy 85/65/40
#   + cfq 500/60/0; powersave 75/55/60 + cfq 500/0. Boost leftovers
#   (default_pwrlevel 4, group 140/120, walt 0) reverted in the next
#   profile-change apply. idle_timer re-audited -> framework-owned
#   (libqti-perfd), stays forbidden; adrenoboost absent on stock.
# - v0.11.3: reconcile ALSO fires on every profile-id change (worker
#   run_once), not just on force — old-profile keys (core_ctl.max_cpus,
#   cpusets) revert to stock.json values. Device proof: sleep -> game
#   move healed max_cpus 2->6 and the cpusets 0-3 -> 0-1 in one apply.
# - v0.11.1: profile identities differ from stock by design. balance =
#   stock-plus (hispeed little 1324@85, core_ctl.min_cpus 2); powersave =
#   powersave governors + caps 1248/1324 + GPU lvl 5; sleep deeper at
#   screen-off (p0.max 1017k, p6.max 1324k); boost ALWAYS writes
#   policy6.scaling_max_freq 2304000 (was absent -> jank burst could not
#   leave a powersave/sleep cap). This is the value set from the F13
#   bench audit (game untouched)
```

## Device quirks (save yourself hours)

- `su -c` cannot use pipes/quotes in the command string (use plain argv-style
  commands); `dumpsys`/binder is unusable from the app's su context — logcat
  (kernel buffer) is the reliable channel.
- The daemon runs as root: it reads/writes `/sys` directly and calls
  `/system/bin/settings` for MIUI mode flags.
- `logcat -v epoch` prints `epoch.seconds` as the first token — freshness is
  arithmetic; never reintroduce `MM-DD HH:MM:SS` parsing (the old Kotlin code
  had a real 1970 bug that silently killed the stream).
- MIUI Ultra saver freezes the app itself; the retire path (restore + stop)
  runs before the freeze lands.
- `scaling_max_freq` (FreqMax) and `scaling_min_freq` (FreqMin) accept a
  stricter external cap/floor as in-sync — that is thermal/QoS winning, by
  design. Do not "fix" this to exact match.
- `workqueue.power_efficient` is kernel read-only (0444) and is intentionally
  absent from the catalog; never add it.
- The notification channel id stays `automation` (renaming would orphan the
  channel); the legacy `dynamic_profile.xml` prefs file stays on disk as a
  downgrade fallback.
