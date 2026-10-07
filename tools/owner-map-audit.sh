#!/usr/bin/env bash
# owner-map-audit.sh — verify MiFineTune's catalog tiers against a real ROM.
#
# Extracts every node the ROM writes at boot (init.qcom.post_boot.sh) and at
# runtime (perf HAL configs) and asserts:
#   1. no FREE-tier catalog path is written by the ROM (post_boot or perf XML)
#   2. cpu-policy paths are normalized (policy0 <-> cpu0/cpufreq) before match
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

[ -f "$POSTBOOT" ] || { echo "missing: $POSTBOOT"; exit 2; }
[ -d "$PERF_DIR" ] || { echo "missing: $PERF_DIR"; exit 2; }

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

CATALOG_FILE="$TMPDIR_AUDIT/catalog.json" \
WRITES_FILE="$TMPDIR_AUDIT/writes.txt" \
python3 - <<'PY'
import json, os, re, sys

catalog = json.load(open(os.environ["CATALOG_FILE"]))
rom_writes = set(open(os.environ["WRITES_FILE"]).read().split())

def normalize(p: str) -> str:
    p = re.sub(r"/sys/devices/system/cpu/cpufreq/policy(\d+)",
               r"/sys/devices/system/cpu/cpu\1/cpufreq", p)
    return p

rom_norm = {normalize(w) for w in rom_writes if "*" not in w}

violations, free_checked, baseline = [], 0, 0
for e in catalog:
    path = normalize(e["path"])
    written = any(w == path or w.startswith(path + "/") or path.startswith(w + "/")
                  for w in rom_norm)
    if e["tier"] == "free":
        free_checked += 1
        if written:
            violations.append(f"FREE but written by ROM: {e['key']} -> {e['path']}")
    else:
        baseline += 1

print(f"catalog: {len(catalog)} entries ({free_checked} free checked, {baseline} baseline)")
if violations:
    print("VIOLATIONS:")
    for v in violations:
        print("  " + v)
    sys.exit(1)
print("OK: no FREE-tier node is written by this ROM (post_boot + perf HAL)")
PY
