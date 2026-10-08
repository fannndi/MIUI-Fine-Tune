# Changelog

## v0.7.0 — observability (2026-10-08)

Read-only telemetry, transition history and an in-app Diagnostics screen —
the foundation for the env-aware guards (v0.7.x) and everything after.

- **Env sampler** (`engine/env.rs` + `daemon/env.rs`): 30 s thread reading
  battery %/status/temp, the best CPU and GPU thermal zones (resolved by
  `type` at startup), and Adreno GPU busy; forwarded as `env` events on
  change. `MIFINETUNE_SYSFS_ROOT` points host tests at a fake tree.
- **Transition history** (`daemon/stats.rs`): one entry per real profile
  switch (from/to/reason/battery/temp) persisted atomically to
  `stats.json` (500-entry cap, survives daemon restarts), served via `stats`.
- **IPC**: new commands `diag` (daemon health: pid, uptime, watchers, holds,
  config, env, history size) and `stats`; new events `env`/`diag`/`stats`.
- **App**: `/ui/DiagnosticsScreen.kt` (daemon health, live environment,
  transition timeline, relayed daemon log with a 200-line in-memory ring);
  Diagnostics row on Home with a live battery/temp/GPU summary.
- **Doctor v2**: env telemetry checks, future-knob surface probe (f2fs /
  devfreq / kgsl / read-head), refresh-rate key, logcat `-v epoch` format.
- **Tests**: 92 unit + 4 host E2E (new: fake-sysfs daemon test asserting
  `env`/`diag`/`stats` end to end).
- **Device E2E (POCO X3)**: doctor v2 all green; diagnostics screen shows
  live battery/thermal/GPU, uptime, watchers, holds; `stats.json` records
  `sleep -> game -> powersave` with battery/temp and survives restarts.

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
