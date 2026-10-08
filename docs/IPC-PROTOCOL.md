# IPC Protocol

The app and the Rust daemon talk **JSON-lines over stdio**: one JSON object
per line, UTF-8, `\n`-terminated.

- **stdin** — commands (app → daemon)
- **stdout** — events (daemon → app)
- **stderr** — human logs; the app relays them to logcat under the
  `MiFineTune` tag (E2E greps depend on this)

Transport notes:

- The daemon is spawned once: `su -c "miui-ft serve --state-dir DIR --config FILE"`.
- Closing stdin (EOF) is the **shutdown signal** — the daemon releases bridge
  holds, emits `bye`, and exits. The app kills the process only after a grace
  period.
- Invalid/oversized lines are logged (`ipc: bad command: ...`) and skipped —
  a protocol mismatch must never crash the daemon.
- `version` is carried by the `hello` event (currently `1`).

## Commands (app → daemon)

| Command | Payload | Meaning |
|---|---|---|
| `hello` | — | handshake; daemon answers with a `state` snapshot |
| `ping` | — | liveness; daemon answers `pong` |
| `shutdown` | — | graceful exit (service being destroyed on purpose) |
| `config_changed` | — | `config.json` was rewritten; reload + re-evaluate now |
| `screen` | `on: bool, locked: bool` | screen/keyguard state (broadcast + 3 s reconcile) |
| `user_present` | — | keyguard dismissed |
| `fg` | `pkg: string` | raw foreground event (forwarded only if the app watches; normally the daemon watches itself) |
| `seed` | `pkg: string\|null` | wake/unlock seed (peeked package) |
| `mw` | `active: bool, other: string\|null` | multi-window state (normally daemon-side) |
| `ultra` | `on: bool` | MIUI Ultra battery saver broadcast |
| `dnd_access` | `granted: bool` | the app reports whether Do Not Disturb access is granted (gates the per-app DND bridge) |
| `set_base` | `profile: string` | manual card tap: apply now + treat as universal base |
| `restore` | — | service-off: drop pending applies, restore stock, release holds |
| `diag` | — | diagnostics snapshot -> `diag` event (health, env, holds) |
| `stats` | — | transition history -> `stats` event |

Examples:

```json
{"cmd":"hello"}
{"cmd":"screen","on":true,"locked":false}
{"cmd":"set_base","profile":"game"}
{"cmd":"restore"}
{"cmd":"diag"}
{"cmd":"stats"}
```

The app only sends `screen` / `user_present` / `ultra` / `config_changed` /
`set_base` / `restore` in normal operation — the other commands exist for
compatibility, tests, and future app-side watchers.

## Events (daemon → app)

| Event | Payload | Meaning |
|---|---|---|
| `hello` | `version: int, pid: int` | first event after startup |
| `pong` | — | reply to `ping` |
| `decision` | `trigger, action, profile?, reason?` | what the arbiter decided (`action`: `none`/`apply`/`retire`) |
| `applied` | `profile, reason, src_pkg?, ok, wrote, verified, failed, ms, settle_ms` | an apply finished (or was already in place) |
| `state` | `state: {...}` | full runtime snapshot (emitted when anything visible changes) |
| `env` | `env: {...}` | environment sample (battery / thermal / GPU busy), emitted on change |
| `diag` | `diag: {...}` | diagnostics reply (health, env, holds) |
| `stats` | `entries: [...]` | transition history reply (oldest first) |
| `bridge` | `msg: string` | MIUI bridge timeline entry (also relayed to logcat) |
| `game_mode_conflict` | `pkg: string` | MIUI Game Booster is still boosting a mapped game |
| `dnd` | `mode: "priority"\|"total"` (absent = release) | the app must apply/restore DND through the official interruption-filter API |
| `restored` | `ok, wrote, verified, failed` | service-off restore finished |
| `retired` | `ok, wrote, verified, failed` | Ultra saver: restore finished, app must disable + stop |
| `error` | `msg: string` | non-fatal error (daemon keeps running) |
| `bye` | — | graceful exit (stdin closed or shutdown) |

The `state` snapshot:

```json
{"event":"state","state":{
  "screen_on": true, "locked": false,
  "multi_window": false, "second_window": null,
  "foreground": "com.YoStarEN.AzurLane",
  "active": "game", "reason": "app", "src_pkg": "com.YoStarEN.AzurLane"
}}
```

`reason` is a raw machine label; the app resolves display text:

| Raw reason | Display |
|---|---|
| `base` | base |
| `screen off` | screen off |
| `multi-window` | multi-window |
| `app` | app label from `src_pkg` (PackageManager) |
| `MIUI saver` | MIUI saver |

Optional fields are omitted when absent (`skip_serializing_if`), never `null`
— parse with "missing = none".

The `env` sample (read-only telemetry; absent fields = node unavailable):

```json
{"event":"env","env":{
  "battery_pct": 85, "charging": true, "battery_temp_c": 32.0,
  "cpu_temp_c": 38.8, "gpu_temp_c": 41.2, "gpu_busy_pct": 3
}}
```

The `diag` reply (subset shown; `config` mirrors config.json):

```json
{"event":"diag","diag":{
  "version": 1, "pid": 1234, "uptime_s": 120,
  "state_dir": "/data/adb/mifinetune",
  "config_path": "/data/data/com.mifinetune/files/config.json",
  "config": {"enabled":true,"dynamic":true,"base_profile":"powersave", "...": "..."},
  "env": {"battery_pct": 85},
  "screen_on": true, "locked": false, "multi_window": false,
  "foreground": "com.miui.home", "active": "powersave", "reason": "base",
  "watchers": {"fg": true, "mw": true},
  "holds": {"perf_held": false, "perf_saved": "middle", "saver_held": false, "saver_saved": false},
  "stats_len": 12
}}
```

The `stats` reply (oldest first, capped at 500; `t` is epoch seconds —
display only, all decision timing is monotonic):

```json
{"event":"stats","entries":[
  {"t":1791466685,"from":"sleep","to":"game","reason":"app","battery":100,"temp_c":41.1},
  {"t":1791466692,"from":"game","to":"powersave","reason":"base","battery":100,"temp_c":41.1}
]}
```

`to` is a profile id or `"stock"` (restore back to factory values).

## Sequence sketches

Startup:

```
app  → {"cmd":"hello"}
dmn  ← {"event":"hello","version":1,"pid":1234}
dmn  ← {"event":"state", ...}
app  → {"cmd":"screen","on":true,"locked":false}
dmn  ← {"event":"decision","trigger":"wake", ...}
dmn  ← {"event":"applied","profile":"balance","reason":"base", ...}
```

Foreground switch (daemon-side watcher):

```
dmn  ← {"event":"decision","trigger":"event","action":"apply","profile":"game","reason":"app"}
dmn  ← {"event":"applied","profile":"game","reason":"app","src_pkg":"com.YoStarEN.AzurLane", ...}
dmn  ← {"event":"bridge","msg":"MIUI perf mirror ON (game)"}
dmn  ← {"event":"state","state":{...}}
```

Service off:

```
app  → {"cmd":"restore"}
dmn  ← {"event":"restored","ok":true,"wrote":28,"verified":28,"failed":0}
app  → (closes stdin)
dmn  ← {"event":"bye"}
```

Ultra saver retire:

```
app  → {"cmd":"ultra","on":true}
dmn  ← {"event":"decision","trigger":"ultra","action":"retire"}
dmn  ← {"event":"retired","ok":true,"wrote":28,"verified":28,"failed":0}
app  → (sets config.enabled=false, stops service, closes stdin)
dmn  ← {"event":"bye"}
```

## Files shared between app and daemon

| File | Writer | Reader | Content |
|---|---|---|---|
| `filesDir/config.json` | app (atomic) | daemon (start, hint, mtime) | user intent: enabled, dynamic, base_profile, `app_map` (legacy mirror), `app_profiles` (per-app mapping + bypass_charge + dnd), `bypass_floor_pct`, sync flags, guards (`guard_battery`/`battery_floor_pct`, `guard_thermal`/`thermal_ceiling_c`), `maintenance`, `charge_limit`/`charge_limit_pct`, `jank_boost` |
| `/data/adb/mifinetune/holds.json` | daemon (atomic) | daemon (recovery) | bridge restore points (crash-safe) |
| `/data/adb/mifinetune/state.json` | daemon (via engine) | both (CLI status) | active profile, last mode |
| `/data/adb/mifinetune/snapshot.json` | daemon (via engine) | daemon | stock values for restore |
| `/data/adb/mifinetune/stats.json` | daemon (atomic) | app (`stats` command) | transition history, capped at 500 entries |
