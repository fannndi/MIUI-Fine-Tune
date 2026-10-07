#!/usr/bin/env bash
# owner-map-audit.sh — verify MiFineTune's catalog tiers against a real ROM
# and against the perf HAL / framework runtime writer list.
#
# Checks:
#   1. no FREE-tier catalog path is written at boot (init.qcom.post_boot.sh)
#      or by the perf HAL configs (vendor/etc/perf/*.xml, powerhint.xml)
#   2. no FREE-tier catalog path matches tools/perf-hal-runtime-writers.txt
#      (runtime writers: libqti-perfd.so strings, netd, XML major groups)
#   3. cpu-policy paths are normalized (policy0 <-> cpu0/cpufreq) before match
#   4. Baseline overlap with runtime writers is reported (informational only)
#
# usage: tools/owner-map-audit.sh <unpacked-rom-dir>
#   e.g. tools/owner-map-audit.sh ~/Downloads/MIO-KITCHEN-*/miui_SURYAGlobal_*_10.0
#
# Exit 0 = Owner Map consistent with this ROM. Run after every ROM/kernel update.
set -euo pipefail

ROM="${1:?usage: owner-map-audit.sh <unpacked-rom-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
POSTBOOT="$ROM/vendor/bin/init.qcom.post_boot.sh"
PERF_DIR="$ROM/vendor/etc/perf"
POWERHINT="$ROM/vendor/etc/powerhint.xml"
RUNTIME="$ROOT/tools/perf-hal-runtime-writers.txt"

[ -f "$POSTBOOT" ] || { echo "missing: $POSTBOOT"; exit 2; }
[ -d "$PERF_DIR" ] || { echo "missing: $PERF_DIR"; exit 2; }
[ -f "$RUNTIME" ] || { echo "missing: $RUNTIME"; exit 2; }

TMPDIR_AUDIT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_AUDIT"' EXIT

echo "== dumping catalog =="
(cd "$ROOT/core" && cargo run -q -- catalog) > "$TMPDIR_AUDIT/catalog.json"

echo "== extracting ROM write targets =="
grep -ohE '/(proc|sys|dev)/[A-Za-z0-9_./,-]+' "$POSTBOOT" > "$TMPDIR_AUDIT/writes.txt" || true
grep -ohE '/(proc|sys|dev)/[A-Za-z0-9_./,@-]+' "$PERF_DIR"/*.xml "$POWERHINT" 2>/dev/null \
    >> "$TMPDIR_AUDIT/writes.txt" || true
sort -u "$TMPDIR_AUDIT/writes.txt" -o "$TMPDIR_AUDIT/writes.txt"
echo "rom write targets: $(wc -l < "$TMPDIR_AUDIT/writes.txt")"
echo "runtime writer entries: $(grep -c -v -E '^[[:space:]]*(#|$)' "$RUNTIME" || true)"

CATALOG_FILE="$TMPDIR_AUDIT/catalog.json" \
WRITES_FILE="$TMPDIR_AUDIT/writes.txt" \
RUNTIME_FILE="$RUNTIME" \
python3 - <<'PY'
import json, os, re, sys

catalog = json.load(open(os.environ["CATALOG_FILE"]))
rom_writes = set(open(os.environ["WRITES_FILE"]).read().split())

runtime = []  # (compiled regex, raw pattern)
for line in open(os.environ["RUNTIME_FILE"]):
    line = line.split("#", 1)[0].strip()
    if not line:
        continue
    pat = re.escape(line).replace("%d", r"\d+").replace("%s", r"\d+")
    runtime.append((re.compile("^" + pat + "(/|$)"), line))

def normalize(p: str) -> str:
    # cpu policy symlink equivalence: .../cpufreq/policy0 == .../cpu0/cpufreq
    return re.sub(r"/sys/devices/system/cpu/cpufreq/policy(\d+)",
                  r"/sys/devices/system/cpu/cpu\1/cpufreq", p)

rom_norm = {normalize(w) for w in rom_writes if "*" not in w}

violations, infos, free_checked, baseline = [], [], 0, 0
for e in catalog:
    path = normalize(e["path"])
    boot_written = any(w == path or w.startswith(path + "/") or path.startswith(w + "/")
                       for w in rom_norm)
    rt_hits = [raw for rx, raw in runtime if rx.match(path)]
    if e["tier"] == "free":
        free_checked += 1
        if boot_written:
            violations.append(f"FREE but boot-written by ROM: {e['key']} -> {e['path']}")
        if rt_hits:
            violations.append(f"FREE but runtime-written [{rt_hits[0]}]: {e['key']} -> {e['path']}")
    else:
        baseline += 1
        if rt_hits:
            infos.append(f"{e['key']} -> {e['path']}  [{rt_hits[0]}]")

print(f"catalog: {len(catalog)} entries ({free_checked} free checked, {baseline} baseline)")
if infos:
    print(f"baseline overlap with runtime writers ({len(infos)}) — coexist, drift-guard protected:")
    for i in infos:
        print("  ~ " + i)
if violations:
    print("VIOLATIONS:")
    for v in violations:
        print("  " + v)
    sys.exit(1)
print("OK: no FREE-tier node is written at boot or at runtime by this ROM")

# --- candidates for future exploration ---------------------------------
# ROM write targets that are NOT in the catalog at all: every path the ROM
# touches is either framework-owned (ignore) or a candidate knob that has
# never been audited. Curated prefixes keep the list actionable.
catalog_paths = {normalize(e["path"]) for e in catalog}
interesting = ("/proc/sys/kernel/", "/proc/sys/vm/", "/proc/sys/net/",
               "/dev/stune/", "/dev/cpuset/", "/queue/iosched/")
cand = sorted(w for w in rom_norm
              if w.startswith(interesting)
              and w not in catalog_paths
              and not any(w.startswith(p) for p in [
                  # known framework-owned zones (never candidates)
                  "/proc/sys/kernel/sched_boost", "/proc/sys/kernel/sched_lib",
                  "/proc/sys/kernel/sched_freq_aggregate",
                  "/proc/sys/vm/swappiness", "/proc/sys/vm/min_free_kbytes",
                  "/proc/sys/vm/page-cluster", "/proc/sys/vm/watermark_boost_factor",
                  "/proc/sys/vm/swap_ratio",
                  "/dev/cpuset/game", "/dev/cpuset/gamelite", "/dev/cpuset/vr",
                  "/dev/cpuset/audio-app", "/dev/cpuset/camera-daemon",
                  "/dev/cpuset/restricted", "/dev/stune/rt", "/dev/stune/audio-app",
              ]))
if cand:
    print(f"candidates not in catalog ({len(cand)}) — audit tier before ever using:")
    for c in cand:
        print("  ? " + c)
PY
