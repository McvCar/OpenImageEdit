#!/usr/bin/env bash
# 修复 / 油漆 / 图层旋转的无头探针：每个用例跑一份测试图副本，跑完交给
# scripts/verify_paint.py 断言像素。
set -u
export PATH="/d/Qt/6.8.3/msvc2022_64/bin:$PATH"
cd "$(dirname "$0")/.." || exit 1

EXE=./target/debug/imageedit.exe
T="C:/Users/Admin/AppData/Local/Temp"
SRC="$T/paint_in.png"

run() { # run <out> <script>
    local out="$1" script="$2"
    cp "$SRC" "$out"
    echo "=== $out  <=  $script"
    "$EXE" "$out" "--script=$script" --auto-quit 2>&1 \
        | grep -E '"event":"(script-done|bucket-fill|gradient|spot-heal|heal-clone|patch-fill|rotate-layer|script-unknown)"'
}

run "$T/p_bucket.png"   "bucket:5,5,32,1,0,0,255,100;save"
run "$T/p_grad.png"     "gradient:0,0,199,0,0,0,FF0000,0000FF,100;save"
run "$T/p_grad_rad.png" "gradient:100,100,100,10,1,0,FFFFFF,000000,80;save"
run "$T/p_spot.png"     "spot:150,150,30,100;save"
run "$T/p_heal.png"     "heal:30,160,40,110,150,150,150;save"
run "$T/p_patch.png"    "patch-fill:130,130,40,40,0,-60,2;save"
run "$T/p_rot.png"      "extract:20,20,100,100;rotate-layer:0,30,1;save"
run "$T/p_undo.png"     "bucket:5,5,32,1,0,0,255,100;undo;save"
run "$T/p_sel.png"      "sel-rect:0,120,200,200;bucket:5,5,32,1,0,0,255,100;save"
echo "=== done"
