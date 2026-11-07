#!/usr/bin/env bash
# P0-0 基线回归：变换数学单测 + 纯数学探针。
# 不启动 Qt / QML，不依赖图片素材，可在接线前反复跑。
#
#   bash scripts/transform_probe.sh
#
# 若装好 debug 版，也可用主程序同名开关核对输出是否逐字一致：
#   PATH=/d/Qt/6.8.3/msvc2022_64/bin:$PATH ./target/debug/imageedit.exe --probe-transform
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== [1/2] cargo test -p imageedit-core =="
cargo test -p imageedit-core

echo "== [2/2] --probe-transform =="
out="$(cargo run -q -p imageedit-core --example transform_probe -- --probe-transform)"
printf '%s\n' "$out"

grep -q '"event":"probe-transform-done","version":1' <<<"$out" \
  || { echo "FAIL: 缺少探针汇总行"; exit 1; }
grep -q '"all_ok":true' <<<"$out" \
  || { echo "FAIL: 探针自检未通过"; exit 1; }
if grep -Eq '"(bounds_ok|hit_ok|serial_ok|panel_ok|bake_ok|all_ok)":false' <<<"$out"; then
  echo "FAIL: 有 case 的不变量为 false"
  exit 1
fi

# 固定基线值：接线前后这几个数必须一模一样，防止「顺手改期望值」。
grep -q '"case":"identity","matrix":\[1.000000,0.000000,0.000000,1.000000,0.000000,0.000000\]' <<<"$out" \
  || { echo "FAIL: identity 矩阵漂移"; exit 1; }
grep -q '"case":"rotate-90".*"bounds":\[10.000000,-10.000000,80.000000,100.000000\]' <<<"$out" \
  || { echo "FAIL: 90° 外接框漂移"; exit 1; }
grep -q '"case":"scale-out-in".*"bounds":\[0.000000,0.000000,100.000000,80.000000\]' <<<"$out" \
  || { echo "FAIL: 先缩后放外接框漂移（疑似又把变换烘焙进位图）"; exit 1; }
grep -q '"case":"panel-pivot","panel_ok":true,"w":200.000000' <<<"$out" \
  || { echo "FAIL: 数值面板 W 回写/参考点不变量失败"; exit 1; }
grep -q '"case":"bake-equivalence","bake_ok":true,"max_diff":0' <<<"$out" \
  || { echo "FAIL: 烘焙后的 identity 位图被二次插值（§2.4 陷阱 2）"; exit 1; }

echo "PASS: P0-0 变换数学基线一致"
