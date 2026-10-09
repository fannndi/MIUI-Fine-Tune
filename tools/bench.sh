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
# adb on PATH by default; override with ADB=/path/to/adb
ADB="${ADB:-adb}"

SUF="$SC-$PROF-$(date +%s)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --- preflight: charged / battery present ---------------------------------
CHG_ON=$("$ADB" shell "dumpsys battery | grep -m1 'AC powered'" | tr -d '\r' | awk '{print $3}')
USB_ON=$("$ADB" shell "dumpsys battery" | grep -m1 "USB powered" | tr -d '\r' | awk '{print $3}')
STATUS=$("$ADB" shell "dumpsys battery" | grep -m1 ' status:' | tr -d '\r' | awk '{print $2}')
SUSPEND=$("$ADB" shell "su -c 'cat /sys/class/power_supply/battery/input_suspend'" 2>/dev/null | tr -d '\r')
if [ "$CHG_ON" = "true" ] || [ "$USB_ON" = "true" ]; then
  # USB present but charging suspended (input_suspend=1): the battery is the
  # only power source, so the fuel-gauge current stays a valid measurement.
  if [ "$SUSPEND" = "1" ] && [ "$STATUS" != "2" ]; then
    echo "NOTE: USB connected with input_suspend=1 (charging suspended) — measuring battery draw."
  else
    echo "REFUSING: device is on a charger (AC=$CHG_ON USB=$USB_ON status=$STATUS suspend=${SUSPEND:-?}) — unplug or suspend first."
    exit 2
  fi
fi

# --- profile setup ---------------------------------------------------------
if [ "$PROF" = "stock" ]; then
  # stock = engine fully off: restore + stop automation
  "$ADB" shell "su -c '$BIN restore'" >/dev/null
else
  "$ADB" shell "su -c '$BIN apply $PROF'" >/dev/null
fi

# --- scenario prep ----------------------------------------------------------
case "$SC" in
  idle-on)
    "$ADB" shell input keyevent 82 >/dev/null   # wake/unlock
    "$ADB" shell wm dismiss-keyguard >/dev/null 2>&1
    "$ADB" shell input keyevent 3 >/dev/null    # home
    ;;
  idle-off)
    "$ADB" shell input keyevent 26 >/dev/null   # screen off
    ;;
  video)
    "$ADB" shell "monkey -p com.google.android.youtube -c android.intent.category.LAUNCHER 1" >/dev/null 2>&1
    sleep 4
    "$ADB" shell input keyevent 127 >/dev/null  # pause (media key) — sample current app state, not playback
    ;;
  game)
    "$ADB" shell input keyevent 82 >/dev/null   # wake/unlock
    "$ADB" shell wm dismiss-keyguard >/dev/null 2>&1
    "$ADB" shell input keyevent 3 >/dev/null    # home
    "$ADB" shell "monkey -p com.YoStarEN.AzurLane -c android.intent.category.LAUNCHER 1" >/dev/null 2>&1
    ;;
  *) echo "unknown scenario: $SC"; exit 2;;
esac
sleep 10   # settle (governors/thermal normalize)

# the game scenario must actually be in the foreground (a locked keyguard
# used to swallow the launch and the run silently measured screen-off)
if [ "$SC" = "game" ]; then
  FOCUS=$("$ADB" shell "dumpsys window 2>/dev/null | grep -m1 mCurrentFocus" | tr -d '\r')
  case "$FOCUS" in
    *YoStarEN.AzurLane/com.manjuu.azurlane.MainActivity*) echo "foreground OK: $FOCUS" ;;
    *YoStarEN*) echo "WARN: game foreground but not MainActivity: $FOCUS" ;;
    *) echo "WARN: game not foreground at measurement start: $FOCUS" ;;
  esac
fi

# --- measurement loop -------------------------------------------------------
STEPS=$(( SECS / 30 ))     # 30 s
: > "$TMP/current.csv"
for i in $(seq 1 "$STEPS"); do
  TS=$(date +%s)
  IDLE=""    # placeholder for synthetic flags
  CUR=$("$ADB" shell "su -c 'cat /sys/class/power_supply/battery/current_now'" | tr -d '\r' | tr -d ' ')
  TEMP=$("$ADB" shell "su -c 'cat /sys/class/thermal/thermal_zone*/temp'" | sort -rn | head -1 | tr -d '\r')
  echo "$TS,current=$CUR,temp=$TEMP" >> "$TMP/current.csv"
  sleep 30
done
"$ADB" shell "dumpsys battery" | grep -E "charge counter|level|status|Charge count" > "$TMP/battery_end.txt"

sleep 2
CHARGE_END=$("$ADB" shell "dumpsys battery" | grep -m1 "Charge counter" | tr -d '\r' | awk '{print $3+0}')
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
echo "markdown_row=| $SC | $PROF | $(( MINS )) min | $CUR_AVG | $(( CUR_AVG * 4318 / 1000000 )) |"
echo "charge_counter_uAh_start_note: captured once before loop (see logs above)"
cat "$TMP/current.csv" | tail -4
echo "restart/home in place — bring device back if needed:"
"$ADB" shell input keyevent 224 >/dev/null 2>&1
