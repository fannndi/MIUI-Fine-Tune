# MiFineTune

MIUI-harmonized performance profiles for the **POCO X3 NFC** (surya / sm6150 /
MIUI 12 / Android 10, root via APatch).

MiFineTune tunes only the parameters MIUI itself leaves alone. It never fights
the framework, never touches SELinux, and always keeps a stock restore path.

- **Rust engine + daemon** (`core/`, binary `miui-ft`) — the only writer of
  tuning parameters. Runs as a root daemon driven by the app.
- **Kotlin/Compose app** — UI and Android lifecycle wiring only. No tuning
  logic lives in Kotlin.

## The four components

1. **Profiles** — three cards (Power Save / Balance / Game) plus a hidden
   `sleep` profile. Tap a card = apply now + it becomes the universal **base**.
2. **Apps Profile** — per-app mapping (e.g. Azur Lane → Game).
3. **Service** — master switch. OFF = full stock restore + stop.
4. **Dynamic Profile** — ON: a mapped app in front overrides the base and the
   MIUI bridge follows it. OFF: the base always wins, mappings are ignored.

## Behavior (verified on device)

| Condition | Result |
|---|---|
| Fresh install / Service OFF | Stock (no intervention) |
| Normal app (WhatsApp, ...) | Base profile (last tapped card) |
| Mapped app in front (Dynamic ON) | Its profile, ~1 s via event system |
| Leaving a mapped app | Back to base |
| Mapped app in front (Dynamic OFF) | Base — mapping ignored |
| Screen off (~10 s grace) | `sleep` |
| Unlock | base / mapped app (per Dynamic) |
| Split screen / floating window | Balance (fixed, overrides mapping) |
| MIUI battery saver (user's own) | Base forced to Power Save |
| MIUI Ultra battery saver | Full retire: restore + stop |
| Battery ≤ floor (default 20%, not charging) | Power Save (beats mapping; sleep/lock still win) |
| CPU near the ceiling (default 75 °C) | Game steps down to Balance, releases 5 °C lower |
| Mapped game in front | + MIUI perf mirror + 120 Hz refresh |
| Power Save-mapped app in front | + MIUI battery saver + 60 Hz refresh |
| Charging + idle (weekly, opt-in) | Bounded f2fs GC window (`dirty_segments` → ≤100) |
| Charge limit (opt-in, default 80 %) | Charging pauses at the limit, resumes 5 % lower; stock switch returns on exit |
| Jank burst ≥ 10 frames (experimental, opt-in) | ~5 s responsive overlay, then back to the normal profile |
| Service OFF | All values written back to stock + daemon exits |

Non-app rules (sleep, multi-window, saver, env guards) keep working with
Dynamic OFF — they are not driven by the app map. The MIUI bridge
(performance mirror, saver follow, refresh follow, game-mode checker) is
app-driven and stops with Dynamic OFF.

Extras: a Diagnostics screen (daemon health, live battery/thermal, 24 h
time-in-profile, relayed daemon log), a Quick Settings tile (service toggle
with the active profile), suggested game mappings, and JSON config
export/import through the system file picker.

## How it works

```
Compose UI ──► HomeViewModel ────────────────┐
   │             │ (apply/restore while off) │ CLI: miui-ft apply ...
   │             ▼                          ▼
   │      DynamicProfileService ──► DaemonClient ──► su ──► miui-ft serve
   │        (FGS + notification)      │  JSON-lines on stdio   (root daemon)
   │                                  │                          │
   └── DynamicProfileState ◄── events ┘                    ┌─────┴─────┐
       (StateFlows for the UI)                             │ arbiter   │
                                                           │ worker    │
   config.json (app-owned, atomic writes) ── read ───────► │ watchers  │
                                                           │ bridge    │
                                                           │ engine    │
                                                           └───────────┘
```

- The daemon reads `config.json`, watches foreground/multi-window via its own
  `logcat -v epoch` streams, decides (pure arbiter), applies through the
  in-process engine, and mirrors MIUI modes through the bridge.
- The app forwards only Android-only signals (screen on/off, keyguard, ultra
  saver broadcasts) and renders state.
- Full details: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
  [docs/IPC-PROTOCOL.md](docs/IPC-PROTOCOL.md).

## Build

```bash
# host: Rust tests (84 unit + 4 host E2E)
cd core && cargo test

# cross-build arm64 + refresh the app assets (syncCore also runs in Gradle)
ANDROID_HOME=$HOME/Android/Sdk cargo ndk -t arm64-v8a build --release
cp target/aarch64-linux-android/release/miui-ft ../app/src/main/assets/miui-ft

# app
cd .. && ./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Toolchain: Gradle 9.7.1 / AGP 9.4.1 / Kotlin 2.4.20 (AGP built-in Kotlin),
Compose BOM 2026.09.00, `compileSdk 37`, JDK 17, Rust stable + cargo-ndk.

## CLI (`miui-ft`)

The same binary serves the daemon and the one-shot CLI (used by tools and the
service-off UI paths):

| Command | Purpose |
|---|---|
| `serve` | stdio daemon (JSON-lines; app-driven lifecycle) |
| `plan <id>` | dry-run: what would be written, per-key status (read-only) |
| `apply <id>` | snapshot → write → read-back verify → set active |
| `verify <id>` | compare live vs profile (drift detection, read-only) |
| `restore` | write the stock snapshot back (consumes it on success) |
| `status` | active profile, snapshot, catalog, device info |
| `doctor` | environment self-check (root, binaries, config, state) |
| `probe` | read every catalog node (JSON) |
| `profiles` / `catalog` | bundled profile pack / full parameter catalog |

State lives in `/data/adb/mifinetune/` (`state.json`, `snapshot.json`,
`holds.json`); the binary is deployed to `/data/local/tmp/mifinetune/miui-ft`.

## Safety guarantees (device-verified)

- **Snapshot** — stock values are recorded before the first write; `restore`
  puts them back byte-for-byte, including ROM quirks (`hispeed 1324600`).
- **Read-back verify** — every write is verified; a mismatch is a failure
  (no false green), with one transient retry for MIUI/thermal races.
- **Forbidden-path guard** — framework-owned nodes (thermal, perf locks,
  charge, LMK/zram, game cpusets, SELinux) are rejected on every write path,
  even if a profile names them (`core/src/engine/catalog/forbidden.rs`).
- **Harmony rules** — a stricter external cap (thermal) or floor (QoS) is
  treated as "framework wins", not drift; see [docs/ROM-HARMONY.md](docs/ROM-HARMONY.md).
- **Atomic state writes** — snapshot/state/holds/config are written with
  tmp+rename; a crash never corrupts them.

## Docs

| File | Content |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | layers, daemon threads, data flow, precision policy |
| [docs/IPC-PROTOCOL.md](docs/IPC-PROTOCOL.md) | command/event schema, lifecycle |
| [docs/ROM-HARMONY.md](docs/ROM-HARMONY.md) | node ownership map, kernel invariants, audit findings |
| [AGENTS.md](AGENTS.md) | file map + hard rules for AI agents and contributors |
