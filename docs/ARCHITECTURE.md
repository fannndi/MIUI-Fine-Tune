# Architecture

MiFineTune is three layers with one rule: **the Rust engine is the only code
that writes parameters; everything else asks it to.**

```
┌─ UI (Compose) ───────────────────────────────────────────────┐
│ HomeScreen · AppsProfileScreen · SettingsScreen              │
│ observe StateFlows, send user intent                         │
└───────────────┬──────────────────────────────────────────────┘
                │ HomeViewModel / DynamicProfileViewModel
                ▼
┌─ App shell (Kotlin, no tuning logic) ────────────────────────┐
│ DynamicProfileService  FGS + notification + Android signals  │
│ DaemonClient           su spawn, JSON-lines, stderr relay    │
│ DynamicProfileConfig   config.json writer (atomic)           │
│ FtClient / Tuner       CLI paths used only while service OFF │
└───────────────┬──────────────────────────────────────────────┘
                │ stdio: JSON commands ↓ / events ↑  (su -c miui-ft serve)
                ▼
┌─ Daemon (Rust, root) ────────────────────────────────────────┐
│ main loop   state machine, timers, command dispatch          │
│ arbiter     pure decision table (no IO)                      │
│ worker      settle/supersede coalescing, retry, restore      │
│ watcher     logcat -v epoch: foreground + multi-window       │
│ bridge      MIUI mode hold/restore (settings CLI)            │
│ engine      plan → guarded writes → read-back verify         │
└──────────────────────────────────────────────────────────────┘
```

## Layer contracts

### Engine (`core/src/engine/`)

The tuning core: catalog (what may be written), probe (what is), plan
(what would be), apply/restore/verify (doing it). Every write passes
`catalog::guard_path`; read-back verification is a hard gate. The engine is
usable stand-alone as a CLI (`miui-ft apply game`).

### Daemon (`core/src/daemon/`)

A long-lived root process started once by the app. It owns:

- **The decision loop** — one `evaluate(trigger, fg_override)` function is the
  single decision point (arbiter + enqueue + bridge sync request).
- **The apply worker** — a dedicated thread; a 400 ms settle window collapses
  one app transition's event burst into one apply; newer decisions supersede
  pending ones; a failed apply is retried once; `restore` cancels pending
  applies (never tune after a restore). The worker skips an apply only when
  the target profile is already active AND the job is not forced: config
  reloads, profile-pack changes, explicit user taps and the **first apply of
  every daemon run** re-plan, so a pack update or drift that happened while
  the daemon was down is reconciled (new catalog keys land without a switch).
- **Watchers** — three `logcat -v epoch` streams (events buffer for
  foreground resume, main buffer for the GameBooster multi-window line and
  for `Choreographer: Skipped N frames!` jank lines) plus a one-shot peek
  used to seed wake/unlock decisions.
- **The bridge** — MIUI performance mirror + battery-saver follow + game-mode
  checker, with a hold/restore state machine persisted in `holds.json`.
- **The environment sampler** (`env.rs`) — read-only telemetry (battery,
  thermal zones, GPU busy) into the main loop every 30 s; the app renders it
  live (`env` events) and `diag` reports it.
- **Jank boost** (`evaluate.rs` + the jank watcher, opt-in, experimental)
  — 10+ skipped frames raise the hidden `boost` profile for a few seconds
  (rate-limited), then the supervisor returns to the normal decision. FAS
  without frame tracing: the signal is the framework's own jank log.
- **Charge guard** (`bridge/charge.rs`, opt-in) — pauses charging at the
  configured limit (release 5 % lower; the captured stock value returns on
  release/service-off). Fed by every env sample so it works with the screen
  off; device-verified 1 -> 0 -> 1 with `charge paused/resumed` events.
- **Storage maintenance** (`maintenance.rs`, opt-in) — weekly bounded f2fs
  GC while charging + screen off, mirroring the ROM's own
  `checkpoint_gc` (sleep 50, `gc_urgent=1`, poll `dirty_segments` to ≤100,
  restore; capped at 10 min). Runs on its own thread; every write passes
  the catalog guard.
- **Env-aware guards** — config-gated safety rules evaluated on every
  decision and immediately when the guard verdict flips on a fresh sample:
  - *battery guard*: at/below `battery_floor_pct` while not charging →
    Power Save (beats mapping, saver and multi-window; sleep/lock still win);
  - *thermal guard*: Game (mapped or base) steps down to Balance at
    `thermal_ceiling_c`, releases 5 °C lower (hysteresis; a missing sensor
    keeps the last state). The guard never fights the kernel — it acts
    *below* the framework's own throttle.
- **The transition history** (`stats.rs`) — one bounded, persisted entry per
  real profile switch (`stats.json`), served to the app's dashboard via the
  `stats` command.
- **Timers** — 10 s sleep grace after screen-off, 15 s periodic re-evaluate,
  1 s config mtime poll, 10 s watcher restart backoff, 30 s env sampling.

Thread model (std threads + mpsc, no async runtime):

```
stdin reader ─┐
worker       ─┼─► main loop (owns all state; no locks)
watchers     ─┤        │
peek threads ─┤        ├─► worker channel   (Work::Apply / Work::Restore)
env sampler  ─┘        └─► bridge channel   (SyncCtx, latest-wins)
```

The main loop never blocks on IO: settings execs happen on the bridge thread,
applies on the worker, logcat on the watcher threads. The bridge's
`user_saver` attribution is a lock-free atomic so decisions never wait.

### App shell (`app/src/main/java/com/mifinetune/`)

- `DynamicProfileService` — foreground service: spawns/restarts the daemon,
  forwards Android-only signals (`screen`, `user_present`, `ultra`), maps
  daemon events into `DynamicProfileState` + notifications. It decides nothing.
- `DaemonClient` — process plumbing: `su -c` spawn, one JSON per line,
  stderr relayed to logcat (`MiFineTune` tag) and ring-buffered in
  `DynamicProfileState.logs` for the in-app Diagnostics screen. Closing stdin
  is the shutdown signal; the daemon exits by itself (EOF), the client kills
  only after a grace period.
- `DynamicProfileConfig` — the app is the single writer of `config.json`
  (atomic tmp+rename). It also migrates the old SharedPreferences once.
- `HomeViewModel` — routes user actions: while the daemon is connected,
  apply/restore go through it (events carry real counters for the dialogs);
  with the service off, the CLI paths (`FtClient`/`Tuner`) are used.

## Data flow

### User config (single writer: the app)

```
UI toggle ──► DynamicProfileConfig ──► filesDir/config.json (atomic)
                    │
                    └─► DaemonLink.configChanged()  ──► daemon reloads + evaluates
```

The daemon also polls the file's mtime once per second as a fallback, and a
parse failure keeps the last good config (reported, never fatal).

### Decision → apply

```
foreground event ──► evaluate(trigger)
                        │  arbiter::decide(input)         (pure)
                        ├─ None      → log only
                        ├─ Apply     → worker.enqueue(Job) ──► engine.apply(profile)
                        └─ Retire    → worker Restore{retire} ──► engine.restore
                        └─ bridge sync request ──► bridge thread (settings IO)
applied event ◄── main loop ◄── worker ◄── engine report (wrote/verified/failed)
```

### Service OFF

```
UI switch OFF ──► config.enabled=false ──► daemon `restore` command
               └─ wait for `restored` event ──► stop service ──► stdin EOF
                                                  ──► daemon releases bridge
                                                      holds and exits
```

### Diagnostics (read-only)

```
env sampler (30 s) ──► Msg::Env ──► main loop ──► `env` event (on change)
                                              └─► kept for diag + guards
app `diag`  ──► health snapshot (pid, uptime, watchers, holds, env, config)
app `stats` ──► stats.json history (one entry per real switch)
UI Diagnostics screen renders all three; the daemon log is relayed over
stderr and ring-buffered in the app (no cable needed).
```

### Env-aware guards

```
env sample ──► battery_low? / thermal_high? (hysteresis)
                    │ flip (false->true or true->false)
                    └─► evaluate("env") ──► arbiter guards:
                         battery_low  -> Power Save    (reason "low battery")
                         thermal_high -> Game->Balance (reason "thermal")
```

Both guards are pure inputs to the arbiter (decisions stay in one table);
the hysteresis state machine lives in the daemon loop. Config keys:
`guard_battery`, `battery_floor_pct`, `guard_thermal`, `thermal_ceiling_c`.
Defaults: on / 20 % / on / 75 °C (surya starts kernel throttling around this
band; guarding below it avoids hard steps without losing headroom).

## Harmony invariant

MiFineTune tunes only parameters MIUI itself leaves alone:

- **Free** — no ROM/perf writer; safe to set.
- **Baseline** — boot/transiently written by MIUI; we set a baseline and the
  framework may overlay it (coexist, never fight).
- **Forbidden** — runtime-owned (thermal, perf locks, charge, LMK/zram, game
  cpusets, SELinux); rejected on every write path.

The bridge is the only non-catalog writer, with exactly three `settings`
targets — `Settings.Global low_power` (live saver), the `Settings.System
power_mode` mirror and `Settings.System user_refresh_rate` (refresh follow) —
plus the charge guard's single cataloged node
`battery_charging_enabled` (Baseline, the ROM's own user-facing switch).
Never sysfs beyond that node, never props, never SELinux. The real power
property (`persist.sys.aries.power_profile`) is SELinux-locked and is never
attempted. Details: [ROM-HARMONY.md](ROM-HARMONY.md).

## Precision policy

- Durations use monotonic `Instant`; jobs carry their queue timestamp so
  settle/latency numbers cannot be distorted by wall-clock jumps.
- Logcat is parsed with `-v epoch`: freshness is epoch arithmetic, immune to
  timezone/year bugs (the old Kotlin parser had a real 1970 regression).
- IPC is strict JSON-lines; invalid lines are logged and skipped. Version is
  carried by the `hello` event.
- All state files are written atomically (tmp + fsync/rename).
- Read-back verification is a hard gate; "already active" is decided by engine
  state, never assumed.
- The settle window is measured from the first queued job of a burst and is
  not extended by new arrivals (a burst cannot postpone the switch forever).

## Lifecycle

```
app start ──► deploy binary (assets → /data/local/tmp)
          ──► start FGS ──► spawn daemon (su -c miui-ft serve)
daemon    ──► hello ──► screen cmd ──► (seeded decision)
service destroyed ──► stdin close ──► daemon: release holds + bye + exit
daemon death      ──► client detects EOF ──► supervisor restarts (10 s backoff)
ultra saver       ──► daemon restores + emits `retired` ──► app disables + stops
```

The daemon never outlives the app: the pipe is the lease. An abrupt kill
self-heals on the next start via `holds.json` + `state.json`.
