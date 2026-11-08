#!/usr/bin/env bash
# shot_live.sh <out.png> <img> <script> [extra args…]
# 起一个带 --script 的探针实例（不会 taskkill 别人正在用的实例），
# 截一张窗口图，然后只杀这一个 PID。
set -u
export PATH="/d/Qt/6.8.3/msvc2022_64/bin:$PATH"
cd "$(dirname "$0")/.." || exit 1
OUT="$1"; shift
IMG="$1"; shift
SCRIPT="$1"; shift
PY="C:/Users/Admin/.workbuddy/binaries/python/envs/default/Scripts/python.exe"

before=$(tasklist //FI "IMAGENAME eq imageedit.exe" //FO CSV 2>/dev/null | cut -d'"' -f4 | tr -d '\r' | sort)
LOG="${TMPDIR:-/tmp}/shot_live.log"
./target/debug/imageedit.exe "$IMG" "--script=$SCRIPT" "$@" > "$LOG" 2>&1 &
sleep 4
after=$(tasklist //FI "IMAGENAME eq imageedit.exe" //FO CSV 2>/dev/null | cut -d'"' -f4 | tr -d '\r' | sort)
PID=$(comm -13 <(echo "$before") <(echo "$after") | grep -E '^[0-9]+$' | tail -1)
if [ -z "${PID:-}" ]; then echo "no new process"; exit 1; fi
echo "pid=$PID log=$LOG"
"$PY" scripts/shot_window.py "$PID" "$OUT"
sleep 1
taskkill //F //PID "$PID" >/dev/null 2>&1
