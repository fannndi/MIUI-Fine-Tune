#!/usr/bin/env bash
# display-off-diff.sh — measure what MIUI itself changes when the screen turns off.
#
# Read-only diagnostic: probes the whole 67-node catalog + framework-owned
# nodes (msm_performance QoS, sched_boost, kgsl devfreq), toggles the screen
# with a power-key event, and diffs the values.
#
# Purpose: verify the "display off" event of the perf HAL
# (perfboostsconfig.xml Id 0x1040 -> opcode 0x40000000) does not disturb any
# catalog node before we introduce the Sleep profile.
#
# usage: tools/display-off-diff.sh [screen-off-seconds]   (default 60)
set -euo pipefail

SECS="${1:-60}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN=/data/local/tmp/mifinetune/miui-ft
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

extras() {
    adb shell "su -c 'for f in \
        /sys/module/msm_performance/parameters/cpu_min_freq \
        /sys/module/msm_performance/parameters/cpu_max_freq \
        /proc/sys/kernel/sched_boost \
        /proc/sys/kernel/sched_freq_aggregate \
        /sys/class/kgsl/kgsl-3d0/devfreq/min_freq \
        /sys/class/kgsl/kgsl-3d0/devfreq/max_freq; do \
        v=\$(cat \$f 2>/dev/null || echo NA); echo \"\$f=\$v\"; done'"
}

temps() {
    adb shell "su -c 'for z in /sys/class/thermal/thermal_zone*; do \
        echo \"\$(cat \$z/type 2>/dev/null)=\$(cat \$z/temp 2>/dev/null)\"; done' 2>/dev/null | sort -t= -k2 -rn | head -6"
}

echo "== state sebelum (layar ON) =="
adb shell "su -c '$BIN status'" | python3 -c "import json,sys; d=json.load(sys.stdin); print('active:', d['active'])"
adb shell "su -c '$BIN probe'" > "$TMP/on.json"
extras > "$TMP/extra_on.txt"
temps  > "$TMP/temps_on.txt"

echo "== matikan layar (power key) — tunggu ${SECS} dtk =="
adb shell input keyevent 26
sleep "$SECS"
adb shell "su -c '$BIN probe'" > "$TMP/off.json"
extras > "$TMP/extra_off.txt"
temps  > "$TMP/temps_off.txt"

echo "== nyalakan layar =="
adb shell input keyevent 26

python3 - "$TMP/on.json" "$TMP/off.json" "$TMP/extra_on.txt" "$TMP/extra_off.txt" "$TMP/temps_on.txt" "$TMP/temps_off.txt" <<'PY'
import json, sys

on  = json.load(open(sys.argv[1]))["entries"]
off = json.load(open(sys.argv[2]))["entries"]

print()
print("== katalog (67 node): layar ON vs OFF ==")
changed = 0
for k in on:
    a, b = on[k], off.get(k, {})
    if a.get("value") != b.get("value") or a.get("exists") != b.get("exists"):
        changed += 1
        print(f"  {k}\n    ON : {a.get('value')}\n    OFF: {b.get('value')}")
print(f"  -> {changed} dari {len(on)} berubah")

print()
print("== node framework (read-only observer) ==")
def rd(p):
    d = {}
    for line in open(p):
        line = line.strip()
        if "=" in line:
            k, v = line.split("=", 1)
            d[k] = v
    return d
eo, ef = rd(sys.argv[3]), rd(sys.argv[4])
for k in eo:
    mark = "  ~" if eo[k] != ef.get(k) else "   "
    print(f"{mark} {k}: ON={eo[k]!r} OFF={ef.get(k)!r}")

print()
print("== suhu (top 6, ON vs OFF) ==")
print("  ON :", " | ".join(open(sys.argv[5]).read().split()))
print("  OFF:", " | ".join(open(sys.argv[6]).read().split()))
PY
