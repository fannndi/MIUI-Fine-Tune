#!/usr/bin/env bash
# rom-write-audit.sh — audit the ROM for sysfs/procfs write targets and
# classify them against MiFineTune's ownership rules.
#
# Extracts every `write /sys|/proc` (init .rc) and `echo VALUE > /sys|/proc`
# (shell) target from an unpacked ROM tree, then labels each one:
#   CATALOG   already in core/src/engine/catalog/entries.rs
#   RUNTIME   in tools/perf-hal-runtime-writers.txt (Baseline coexist only)
#   FORBIDDEN under catalog/forbidden.rs prefixes/keys
#   NEW       none of the above -> audit tier evidence before ever using it
#
# With --device it also diffs the boot-written VALUE against the live value:
#   DIFF      live != boot -> an unknown runtime writer exists -> treat as
#             runtime-owned (never Free); add it to the writers list.
#
# Usage: tools/rom-write-audit.sh <unpacked-rom-root> [--device]
#        (the device must be reachable via adb for --device)
set -euo pipefail

ROM=${1:?usage: rom-write-audit.sh <unpacked-rom-root> [--device]}
MODE=${2:-}
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CAT="$ROOT/core/src/engine/catalog/entries.rs"
RW="$ROOT/tools/perf-hal-runtime-writers.txt"
FK="$ROOT/core/src/engine/catalog/forbidden.rs"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

[ -d "$ROM" ] || { echo "not a directory: $ROM" >&2; exit 2; }

# --- 1) collect write targets + boot-written values -------------------------
# rc:  "write <path> <value>"
grep -rhoE 'write +/(sys|proc)[A-Za-z0-9_.,:/@%+-]+ +[^ ]+' "$ROM" --include='*.rc' 2>/dev/null \
  | awk '{print $2"|"$3}' | sort -u > "$TMP/rc_pairs" || true
cut -d'|' -f1 "$TMP/rc_pairs" > "$TMP/rc_paths"
# sh:  "echo <value> > <path>"  (value must be a literal)
grep -rhoE 'echo +[^ >]+ *>+ */(sys|proc)[A-Za-z0-9_.,:/@%+-]+' "$ROM" --include='*.sh' 2>/dev/null \
  | awk '$2 ~ /^[A-Za-z0-9_.-]+$/ {print $4"|"$2}' | sort -u > "$TMP/sh_pairs" || true
cut -d'|' -f1 "$TMP/sh_pairs" > "$TMP/sh_paths"
cat "$TMP/rc_paths" "$TMP/sh_paths" | sort -u > "$TMP/targets"
cat "$TMP/rc_pairs" "$TMP/sh_pairs" | sort -u > "$TMP/pairs"

# --- 2) reference lists -----------------------------------------------------
grep -oE '"/(sys|proc|dev)[A-Za-z0-9_.,:/@%+-]*"' "$CAT" | tr -d '"' | sort -u > "$TMP/catalog"
grep -vE '^(#|$)' "$RW" | sort -u > "$TMP/rw"
awk '/FORBIDDEN_PREFIXES/,/^\];/' "$FK" | grep -oE '"[^"]+"' | tr -d '"' > "$TMP/fk_p"
awk '/FORBIDDEN_KEYS/,/^\];/' "$FK" | grep -oE '"[^"]+"' | tr -d '"' > "$TMP/fk_k"

classify() {
  local p="$1" c f
  while read -r c; do
    [ -z "$c" ] && continue
    case "$p" in "$c"|"$c"*) echo CATALOG; return ;; esac
  done < "$TMP/catalog"
  while read -r f; do
    [ -z "$f" ] && continue
    case "$p" in "$f"|"$f"*) echo RUNTIME; return ;; esac
  done < "$TMP/rw"
  while read -r f; do
    [ -z "$f" ] && continue
    case "$p" in "$f"|"$f"*) echo FORBIDDEN; return ;; esac
  done < "$TMP/fk_p"
  echo NEW
}

: > "$TMP/out"
while IFS= read -r p; do
  [ -z "$p" ] && continue
  echo "$(classify "$p")|$p"
done < "$TMP/targets" > "$TMP/out"

echo "rom-write-audit: $ROM"
printf "targets: %s (rc: %s, sh: %s)\n" \
  "$(wc -l < "$TMP/targets")" "$(wc -l < "$TMP/rc_paths")" "$(wc -l < "$TMP/sh_pairs")"
echo "classified:"
cut -d'|' -f1 "$TMP/out" | sort | uniq -c | sed 's/^ *//'
echo
echo "NEW candidates (audit tier evidence before ever using):"
grep '^NEW|' "$TMP/out" | cut -d'|' -f2 | sed 's/^/  ? /' || true

# --- 3) optional live diff --------------------------------------------------
if [ "$MODE" = "--device" ]; then
  # adb resolver: PATH, then the standard SDK locations
  ADB=$(command -v adb || true)
  if [ -z "$ADB" ]; then
    for c in "${ANDROID_HOME:-}/platform-tools/adb" "${ANDROID_SDK_ROOT:-}/platform-tools/adb" \
             "$HOME/Android/Sdk/platform-tools/adb"; do
      [ -n "${c%/platform-tools/adb}" ] && [ -x "$c" ] && { ADB="$c"; break; }
    done
  fi
  [ -n "$ADB" ] || { echo "adb not found" >&2; exit 2; }
  # some /proc nodes are root-only (e.g. vm/swap_ratio) -> probe via su
  SU=""
  SUFIX=""
  case "$("$ADB" shell "su -c id" 2>/dev/null)" in
    *uid=0*) SU="su -c '"; SUFIX="'" ;;
  esac
  # boot value per path (best effort: first writer wins) -> probe live once
  awk -F'|' '!seen[$1]++ {print $1"|"$2}' "$TMP/pairs" > "$TMP/boot_vals"
  LOOP='while IFS= read -r line; do p=$line; if [ -e "$p" ]; then lv=$(head -c 64 "$p" 2>/dev/null | tr -d "\n"); echo "$p|$lv"; else echo "$p|MISSING"; fi; done'
  cut -d'|' -f1 "$TMP/boot_vals" > "$TMP/boot_paths"
  "$ADB" shell "${SU}${LOOP}${SUFIX}" < "$TMP/boot_paths" > "$TMP/live" 2>/dev/null || true
  if [ -n "${AUDIT_DEBUG:-}" ]; then
    cp "$TMP/live" "$TMP/pairs" /tmp/audit_dbg_$$/ 2>/dev/null || { mkdir -p /tmp/audit_dbg_$$; cp "$TMP/live" "$TMP/pairs" /tmp/audit_dbg_$$/; }
    echo "[debug] files in /tmp/audit_dbg_$$"
  fi
  echo
  echo "live-vs-boot diff (only nodes that EXIST on the device):"
  # NOT-PRESENT = node absent (dead multi-SoC path); VALUE-DIFF = live value
  # not among ROM boot values -> the branch did not run or an unknown
  # runtime writer owns it -> never Free, verify before Baseline.
  awk -F'|' 'FNR==NR {live[$1]=$2; next}
      { k=$1; if (!(k in live)) next;
        l=live[k];
        if (l=="MISSING") { npk[k]=1; next }
        if (l==$2) eq[k]=1; else { diff[k]=1; if (!(k in bf)) bf[k]=$2 } }
      END { n=0; for (k in diff) if (!(k in eq)) {
              printf "  VALUE-DIFF  %-52s boot=%-14s live=%s\n", k, substr(bf[k],1,14), substr(live[k],1,44); n++ }
            s=0; for (k in eq) s++; m=0; for (k in npk) m++
            printf "  (same: %d, not-present: %d, value-diff: %d)\n", s, m, n }' \
      "$TMP/live" "$TMP/pairs"
fi
