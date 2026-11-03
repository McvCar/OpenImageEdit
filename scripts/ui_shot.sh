#!/usr/bin/env bash
# ui_shot.sh <out.png> <img> [args…] —— 起一个探针实例、截图、关掉。
set -u
export PATH="/d/Qt/6.8.3/msvc2022_64/bin:$PATH"
cd "$(dirname "$0")/.." || exit 1
OUT="$1"; shift
IMG="$1"; shift
PY="C:/Users/Admin/.workbuddy/binaries/python/envs/default/Scripts/python.exe"
taskkill //F //IM imageedit.exe >/dev/null 2>&1
./target/debug/imageedit.exe "$IMG" "$@" >/dev/null 2>&1 &
sleep 3
PID=$(tasklist //FI "IMAGENAME eq imageedit.exe" //FO CSV | tail -1 | cut -d'"' -f4)
if [ -z "${PID:-}" ]; then echo "no process"; exit 1; fi
"$PY" scripts/shot_window.py "$PID" "$OUT"
taskkill //F //IM imageedit.exe >/dev/null 2>&1
