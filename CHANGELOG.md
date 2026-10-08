# Changelog

## v0.7.0 — observability + adaptive guards (2026-10-08)

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
- **Tests**: 102 unit + 8 host E2E across three binaries (protocol/decision,
  watcher streams, bridge hold/restore; plus fake-sysfs env/guard E2E).
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
  `Power Save 41m · Sleep 2m · Game 1m`.

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
