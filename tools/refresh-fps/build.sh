#!/usr/bin/env bash
# Rebuild core/assets/refresh_fps.dex from RefreshFps.java.
# Needs a JDK (javac) and Android build-tools (d8).
set -euo pipefail
cd "$(dirname "$0")"
D8=$(ls "$HOME/Android/Sdk/build-tools/"*/d8 2>/dev/null | tail -1)
[ -n "$D8" ] || { echo "d8 not found (Android build-tools)"; exit 1; }
rm -rf out && mkdir out
javac -source 8 -target 8 -d out RefreshFps.java 2>/dev/null
"$D8" --min-api 29 --output out out/mifinetune/RefreshFps.class
cp out/classes.dex ../../core/assets/refresh_fps.dex
echo "ok: core/assets/refresh_fps.dex ($(stat -c%s ../../core/assets/refresh_fps.dex) bytes)"
