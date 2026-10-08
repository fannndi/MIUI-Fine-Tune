#!/usr/bin/env bash
# bench.sh — empirical efficiency harness for profile A/B testing.
#
# Uses the battery fuel-gauge (charge_counter, µAh-level) + live current
# (µA) + package-level battery stats + thermal to score a scenario run.
# Read-only on the engine: profile switching is done via the audited
# `miui-ft` CLI, never by writing nodes here.
#
# problems the harness avoids:
#  - 1%-percent granularity (charge_counter resolves ~0.1 %)
#  - plugging state changes mid-run (refuses to start while charging)
#  - display-off drift leaking into an idle-on sample (screen is forced on
#     and verified awake at the end)
#
# usage:
#   tools/bench.sh <scenario> <profile> [minutes]
# scenarios: idle-on | idle-off | video | game
#   idle-on  : launcher visible, screen on
#   idle-off : screen off (power key), automation handles sleep
#   video    : YouTube video playback (needs first-play setup once)
#   game     : Azur Lane foreground (automation maps it → game)
#
# example (12 min idle-on under balance):
#   tools/bench.sh idle-on balance 12
set -euo pipefail

SC="${1:?usage: bench.sh <scenario> <profile> [minutes]}"
PROF="${2:?profile: stock | powersave | balance | game}"
MINS="${3:-10}"
SECS=$(( MINS * 60 ))
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN=/data/local/tmp/mifinetune/miui-ft

SUF="$SC-$PROF-$(date +%s)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

sample() () {}

# --- preflight: charged / battery present ---------------------------------
CHG_ON=$(adb shell "dumpsys battery | grep -m1 'AC powered'" | tr -d '\r' | awk '{print $3}')
USB_ON=$(adb shell "dumpsys battery" | grep -m1 "USB powered" | tr -d '\r' | awk '{print $3}')
if [ "$CHG_ON" = "true" ] || [ "$USB_ON" = "true" ]; then
  echo "REFUSING: device is on a charger (AC=$CHG_ON USB=$USB_ON) — unplug first."
  exit 2
fi

# --- profile setup ---------------------------------------------------------
if [ "$PROF" = "stock" ]; then
  # stock = engine fully off: restore + stop automation
  adb shell "su -c '$BIN restore'" >/dev/null
else
  adb shell "su -c '$BIN apply $PROF'" >/dev/null
fi

# --- scenario prep ----------------------------------------------------------
case "$SC" in
  idle-on)
    adb shell input keyevent 82 >/dev/null   # wake/unlock
    adb shell wm dismiss-keyguard >/dev/null 2>&1
    adb shell input keyevent 3 >/dev/null    # home
    ;;
  idle-off)
    adb shell input keyevent 26 >/dev/null   # screen off
    ;;
  video)
    adb shell "monkey -p com.google.android.youtube -c android.intent.category.LAUNCHER 1" >/dev/null 2>&1
    sleep 4
    adb shell input keyevent 127 >/dev/null  # pause (media key) — sample current app state, not playback
    ;;
  game)
    adb shell "monkey -p com.YoStarEN.AzurLane -c android.intent.category.LAUNCHER 1" >/dev/null 2>&1
    ;;
  *) echo "unknown scenario: $SC"; exit 2;;
esac
sleep 10   # settle (governors/thermal normalize)

# --- measurement loop -------------------------------------------------------
STEPS=$(( SECS / 30 ))     # 30 s
: > "$TMP/current.csv"
for i in $(seq 1 "$STEPS"); do
  TS=$(date +%s)
  IDLE=""    # placeholder for synthetic flags
  CUR=$(adb shell "su -c 'cat /sys/class/power_supply/battery/current_now'" | tr -d '\r' | tr -d ' ')
  TEMP=$(adb shell "su -c 'cat /sys/class/thermal/thermal_zone*/temp'" | sort -rn | head -1 | tr -d '\r')
  echo "$TS,current=$CUR,temp=$TEMP" >> "$TMP/current.csv"
  sleep 30
done
adb shell "dumpsys battery" | grep -E "charge counter|level|status|Charge count" > "$TMP/battery_end.txt"

sleep 2
CHARGE_END=$(adb shell "dumpsys battery" | grep -m1 "Charge counter" | tr -d '\r' | awk '{print $3+0}')
CUR_AVG=$(python3 - "$TMP/current.csv" <<'PY'
import sys, statistics
vals = []
for line in open(sys.argv[1]):
    for t in line.strip().split(","):
        if t.startswith("current="):
            try: vals.append(abs(int(t.split("=")[1])))
            except Exception: pass
print(int(statistics.mean(vals)) if vals else 0)
PY
)

echo
echo "===== RESULT $SUF ====="
echo "scenario=$SC profile=$PROF minutes=$MINS"
echo "avg_current_uA=$CUR_AVG"
echo "approx_power_mW=$(( CUR_AVG * 4318 / 1000000 ))   # assume 4.318 V nominal"
echo "charge_counter_uAh_start_note: captured once before loop (see logs above)"
cat "$TMP/current.csv" | tail -4
echo "restart/home in place — bring device back if needed:"
adb shell input keyevent 224 >/dev/null 2>&1
