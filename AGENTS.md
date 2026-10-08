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

## File map (one line per file — keep this current)

### Rust (`core/`)

| Path | Responsibility |
|---|---|
| `src/lib.rs` | module index (`daemon`, `engine`, `DEFAULT_PROFILES_JSON`) |
| `src/main.rs` | CLI entry: probe/profiles/plan/apply/restore/verify/status/serve/catalog |
| `profiles.json` | profile pack (embedded via `include_str!`, mirrored to assets) |
| `src/engine/mod.rs` | engine module index + test fixtures hookup |
| `src/engine/catalog/mod.rs` | tier/kind/entry types, `find`, `guard_path` (forbidden-path guard) |
| `src/engine/catalog/entries.rs` | the 84-node registry table (data only) |
| `src/engine/catalog/forbidden.rs` | framework-owned path prefixes + exact keys (never written) |
| `src/engine/probe.rs` | read-only device capture: node values, options, framework evidence |
| `src/engine/profile.rs` | `profiles.json` model + parser |
| `src/engine/plan.rs` | `build_plan`: statuses, kernel-invariant pair checks, write ordering |
| `src/engine/validate.rs` | per-kind value validation (OPP clamping, core_ctl bounds) |
| `src/engine/readback.rs` | read-back comparison rules + snapshot normalization |
| `src/engine/apply/mod.rs` | apply/restore/verify module index + shared report types |
| `src/engine/apply/store.rs` | state dir: atomic JSON, snapshot capture, profile loading |
| `src/engine/apply/write.rs` | guarded writes, kernel-safe ordering, pass-2 re-plan |
| `src/engine/apply/restore.rs` | stock restore (consumes the snapshot on success) |
| `src/engine/apply/verify.rs` | drift detection + verified read-back helper |
| `src/engine/testutil.rs` | shared engine test fixtures (`cfg(test)`) |
| `src/daemon/mod.rs` | daemon main loop: state, timers, command dispatch, supervisor |
| `src/daemon/proto.rs` | IPC wire format (Command/Event) + stdout Publisher |
| `src/daemon/config.rs` | `config.json` cache (app writes; daemon reloads) |
| `src/daemon/arbiter.rs` | pure decision table (ported from Kotlin; all cases tested) |
| `src/daemon/worker.rs` | coalescing apply worker: settle/supersede/retry/restore-cancel |
| `src/daemon/engine_driver.rs` | engine adapter for the worker (in-process, no `su`) |
| `src/daemon/watcher.rs` | logcat watchers (`-v epoch`): foreground + multi-window + peek |
| `src/daemon/bridge.rs` | MIUI bridge: hold/restore state machine, perf/saver/game-check |
| `src/daemon/settings.rs` | `settings` CLI read/write helpers (saver, power_mode) |
| `tests/daemon_smoke.rs` | end-to-end protocol tests: full decision path, EOF exit |

### Kotlin (`app/src/main/java/com/mifinetune/`)

| Path | Responsibility |
|---|---|
| `MainActivity.kt` | single activity, Compose host |
| `core/RootBridge.kt` | `su` exec + binary/profile deploy (assets → device) |
| `core/FtClient.kt` | typed CLI client (JSON parsing) for service-off paths |
| `core/Models.kt` | CLI JSON shapes (status/plan/report/profiles) |
| `core/Tuner.kt` | serialized CLI access + manual drift guard (service-off only) |
| `dynamic/DaemonClient.kt` | daemon spawn, JSON-lines framing, stderr→logcat relay, `DaemonLink` |
| `dynamic/DynamicProfileService.kt` | FGS + notification; forwards screen/keyguard/ultra; maps events to state |
| `dynamic/DynamicProfileConfig.kt` | `config.json` writer (atomic), prefs migration, daemon hints |
| `dynamic/DynamicProfileState.kt` | process-wide StateFlows mirrored from daemon events |
| `dynamic/DeviceContext.kt` | PowerManager/KeyguardManager reader only |
| `dynamic/DynamicProfileBootReceiver.kt` | best-effort service start on boot |
| `ui/HomeScreen.kt` / `HomeViewModel.kt` | profile cards, service/dynamic switches, report dialogs |
| `ui/DynamicProfileViewModel.kt` | apps list + settings toggles controller |
| `ui/AppsProfileScreen.kt` / `SettingsScreen.kt` / `UiBits.kt` / `ProfileLabels.kt` / `theme/` | Compose UI |

### Other

| Path | Content |
|---|---|
| `docs/ARCHITECTURE.md` | layers, daemon thread model, data flow, precision policy |
| `docs/IPC-PROTOCOL.md` | stdio protocol schema + lifecycle |
| `docs/ROM-HARMONY.md` | node ownership map, kernel invariants, audit findings |
| `tools/bench.sh` | on-device benchmark harness (CLI-based) |
| `tools/owner-map-audit.sh` | catalog vs ROM audit (boot + runtime writers) |
| `tools/display-off-diff.sh` | empirical display-off behavior test |
| `tools/perf-hal-runtime-writers.txt` | runtime writer evidence list |

## Hard rules (never break these)

1. **Harmony guard** — the engine writes only Free/Baseline catalog nodes;
   framework-owned paths are rejected by `guard_path` on every write. Never
   add a write path that bypasses the engine guard. Never touch SELinux
   (`/sys/fs/selinux`, `setenforce`, `restorecon`), never attempt `setprop`
   for `persist.sys.aries.power_profile` (SELinux rejects it on this device).
2. **Single writer** — all parameter writes go through the daemon (or the CLI
   when the service is off). The app never writes sysfs/proc directly.
3. **Ownership evidence** — adding a catalog entry requires tier evidence
   (post_boot + perf XML + runtime writers + device); see ROM-HARMONY.md.
4. **Coexist, never fight** — nodes the framework rewrites at runtime are at
   least Baseline; nodes rewritten continuously (e.g. `net.tcp_rmem/wmem`)
   stay out of profiles entirely (there is an automated test for this).
5. **Restore = byte-exact stock** — never "normalize" the snapshot.
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
# Rust (host): 81 unit tests + 2 protocol E2E tests
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
