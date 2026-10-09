# Bench report

Empirical battery-draw comparison of the Device Profiles on the target
device (POCO X3 NFC / surya, MIUI 12, V12.0.7.0), 2026-10-09. Produced by
[`tools/bench.sh`](../tools/bench.sh) — the script is read-only on the
engine: profiles are applied through the audited `miui-ft` CLI, values are
never written by the harness itself.

## Method

- **Metric**: average **battery** current from the fuel gauge
  (`battery/current_now`, µA; sign verified with a controlled
  `input_suspend` test: negative = charging, positive = discharging) plus an
  approximate power figure (× 4.318 V nominal).
- **Sampling**: one read every 30 s for the whole run; the reported value is
  the mean of the absolute samples (10 min → 20 samples).
- **Power source**: the phone ran **on battery** for every run. The USB
  cable stayed attached for `adb`, with charging suspended
  (`input_suspend=1` → the battery is the only source, so the fuel-gauge
  current stays valid; the harness refuses to run while the status is
  `Charging`). Wi-Fi adb is blocked on the bench network (AP client
  isolation), hence the suspended-cable approach.
- **Service state**: the MiFineTune service was **off** for the whole
  session; profiles were applied with `miui-ft apply <profile>`
  (`restore` = stock). No daemon interference.
- **Environment**: screen timeout 30 min (idle-on), screen off (idle-off);
  same radio / brightness / thermal conditions across runs. 10 minutes per
  run, ordered stock → profile so thermal drift works against the profile
  runs, not for them.

## Results

| Scenario | Profile | Duration | avg current (µA) | ≈ power (mW) |
|---|---|---|---|---|
| idle-on (launcher) | stock | 10 min | 211 608 | 913 |
| idle-on (launcher) | powersave | 10 min | 225 341 | 973 |
| idle-on (launcher) | balance | 10 min | 213 042 | 919 |
| idle-off (screen off) | stock | 10 min | 106 521 | 459 |
| idle-off (screen off) | **sleep** | 10 min | **90 606** | **391** |
| game (Azur Lane) | stock | — | _deferred_ | _deferred_ |
| game (Azur Lane) | game | — | _deferred_ | _deferred_ |

## What the numbers say

- **idle-off: sleep is −15 % vs stock** (90.6 mA vs 106.5 mA) — the profile
  identity lands (parked cores via busy thresholds, cfq async batching,
  lowest schedutil floors, p6 min… see `core/profiles.json`).
- **idle-on: all three profiles sit inside a ±6 % band** (stock 211.6,
  balance 213.0, powersave 225.3 mA). A static launcher screen is dominated
  by the display + background sync; the CPU/GPU tunings cannot move it much.
  The interesting deltas (load scenarios: game, video, scroll) are exactly
  what the game scenario was supposed to capture — see below.
- Powersave being the *highest* of the three is within run-to-run noise
  (single 10-minute runs; background sync bursts differ). If it repeats,
  look at its `powersave` governor trade (longer time-on-core vs lower
  voltage rails).

## Deferred: the game scenario

Azur Lane refuses to enter the lobby without a **251.8 MB update** dialog;
`Cancel` exits the game. The two game runs were therefore skipped for this
report (user decision, 2026-10-09).

A first attempt *did* run while the script's `game` prep forgot to
wake/unlock the keyguard — the launch was swallowed, the screen stayed off,
and the runs silently measured screen-off drain (113/128 mA). Those numbers
are **discarded**. `tools/bench.sh` is now fixed accordingly:

- game/video prep does wake → dismiss-keyguard → home → launch;
- after the settle it asserts the foreground is
  `…azurlane.MainActivity` and prints `foreground OK` (or a `WARN`), so a
  swallowed launch can never masquerade as a measurement again.

Re-run plan once the game is able to reach its lobby (no download needed):
`tools/bench.sh game stock 10` then `tools/bench.sh game game 10`, stock
first. Add the two rows here afterwards.
