"""11-07 复测反馈的 5 个问题端到端回归（无需 GUI 操作）。

覆盖：
  Q1 移动中按 Ctrl 应临时关掉吸附（`move-gesture` 透传 mods）；
  Q2 第二次拖动图层后框与内容不许分家（offset 与参数层 center 同步）；
  Q3 旋转后再拉伸：参数层不被烘焙（sw/sh 恒定）、外接框不膨胀、不糊；
  Q4 框选裁剪 / 提取 / 剪切成功后选区必须消失；
  Q5 数值面板与菜单（布局改动无法脚本断言，靠截图与单元测试）。

用法：python scripts/verify_fix1107.py [exe]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = tempfile.gettempdir()
SRC = os.path.join(TMP, "fix1107_in.png")
CTRL = 0x04000000


def fixture(path):
    im = Image.new("RGB", (400, 300), (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, 400, 4):
        d.rectangle([x, 0, x + 1, 299], fill=(0, 0, 0))
    d.rectangle([60, 60, 340, 240], outline=(220, 30, 30), width=3)
    im.save(path)


def events_of(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return out


def run(tag, ops, timeout=180):
    dst = os.path.join(TMP, "fix1107_%s.png" % tag)
    shutil.copyfile(SRC, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    args = [EXE, dst, "--script=" + ops, "--auto-quit"]
    p = subprocess.run(args, capture_output=True, text=True, env=env, timeout=timeout)
    return events_of(p.stdout + p.stderr), dst


def of(ev, name):
    return [e for e in ev if e.get("event") == name]


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  " + detail) if detail else ""))
    if not ok:
        raise SystemExit(1)


def main():
    fixture(SRC)

    # ---- Q4 裁剪 / 提取 / 剪切后选区消失 ----
    ev, _ = run("crop-sel", "sel-rect:20,20,120,100;crop:20,20,120,100;sel-state")
    sel = of(ev, "sel-state")
    check("裁剪后选区消失", bool(sel) and sel[-1]["state"].get("active") is False, repr(sel[-1:]))

    ev, _ = run("extract-sel", "extract:20,20,200,160;sel-state")
    sel = of(ev, "sel-state")
    check("提取图层后选区消失", bool(sel) and sel[-1]["state"].get("active") is False, repr(sel[-1:]))

    ev, _ = run("cut-sel", "cut:20,20,120,100;sel-state")
    sel = of(ev, "sel-state")
    check("剪切像素后选区消失", bool(sel) and sel[-1]["state"].get("active") is False, repr(sel[-1:]))

    # ---- Q1 Ctrl 在移动中临时关闭吸附 ----
    # 图层从 (20,20) 起步，向右拖 170 → 左边缘落在 190，距画布中线 200 只有
    # 10px（> 8 显示像素阈值）—— 再挪 2px 才进吸附区；改用 176 → 196 在阈值内。
    ev, _ = run("snap-on", "extract:20,20,200,160;move-gesture:176,0")
    mv = of(ev, "move-gesture")
    on_ox = mv[-1]["ox"] if mv else None
    check("不按 Ctrl 时移动会吸附到画布中线", on_ox == 200, "ox=%s" % on_ox)

    ev, _ = run("snap-ctrl", "extract:20,20,200,160;move-gesture:176,0,%d" % CTRL)
    mv = of(ev, "move-gesture")
    ctrl_ox = mv[-1]["ox"] if mv else None
    check("按住 Ctrl 时移动不吸附（偏移精确）", ctrl_ox == 196, "ox=%s" % ctrl_ox)
    check("按住 Ctrl 时不产生参考线", bool(mv) and mv[-1]["guides"] == 0,
          "guides=%s" % (mv[-1]["guides"] if mv else None))

    # ---- Q2 连续两次拖动：框（offset）与参数层 center 必须同步 ----
    ev, _ = run("move-twice",
                "extract:20,20,200,160;move-layer:40,30;layer-info:0;"
                "move-layer:40,30;layer-info:0")
    info = of(ev, "layer-info")
    first, second = (info[0], info[1]) if len(info) >= 2 else ({}, {})
    ok = (second.get("ox") == 100 and second.get("oy") == 80
          and abs(second.get("cx", 0) - (100 + second.get("w", 0) / 2.0)) < 0.6
          and abs(second.get("cy", 0) - (80 + second.get("h", 0) / 2.0)) < 0.6)
    check("第二次拖动后中心与 offset 仍一致", ok, "%r -> %r" % (first, second))

    # ---- Q3 旋转 → 拉伸 → 再旋转：参数层不烘焙、外接框不膨胀 ----
    ev, _ = run("rot-scale",
                "extract:20,20,200,160;rotate-layer:0,30;layer-info:0;"
                "scale-layer:300,240;layer-info:0;rotate-layer:0,20;layer-info:0")
    info = of(ev, "layer-info")
    ok_len = len(info) >= 3
    a, b, c = (info[0], info[1], info[2]) if ok_len else ({}, {}, {})
    check("旋转后拉伸仍是参数层（原图尺寸不变）",
          ok_len and b.get("sw") == 200 and b.get("sh") == 160,
          "src=%sx%s" % (b.get("sw"), b.get("sh")))
    check("旋转角度在拉伸后保持", ok_len and abs(b.get("rot", 0) - 30.0) < 0.01,
          "rot=%s" % b.get("rot"))
    # 等比缩放：外接框的长宽比不变（内容不会被压扁）。
    ratio_a = a.get("w", 1) / max(1, a.get("h", 1))
    ratio_b = b.get("w", 1) / max(1, b.get("h", 1))
    check("拉伸旋转过的图层是等比（不压扁）", ok_len and abs(ratio_a - ratio_b) < 0.02,
          "%.3f vs %.3f" % (ratio_a, ratio_b))
    # 再转 20°：累计角度 50°，位图必须由「原图 × 累计角度」重算，而不是把上
    # 一张外接矩形再转一遍（旧实现那样每次 +41% 面积）。判据：外接框尺寸等于
    # 解析式 |cos|·W + |sin|·H。
    import math
    c_rad = math.radians(abs(c.get("rot", 0.0)))
    exp_w = abs(math.cos(c_rad)) * c.get("sw", 0) * c.get("sx", 0) \
        + abs(math.sin(c_rad)) * c.get("sh", 0) * c.get("sy", 0)
    exp_h = abs(math.sin(c_rad)) * c.get("sw", 0) * c.get("sx", 0) \
        + abs(math.cos(c_rad)) * c.get("sh", 0) * c.get("sy", 0)
    check("再旋转后角度累加", ok_len and abs(c.get("rot", 0) - 50.0) < 0.01,
          "rot=%s" % c.get("rot"))
    check("再旋转后外接框等于解析尺寸（没有二次膨胀）",
          ok_len and abs(c.get("w", 0) - exp_w) <= 2 and abs(c.get("h", 0) - exp_h) <= 2,
          "w/h %sx%s vs %.1fx%.1f" % (c.get("w"), c.get("h"), exp_w, exp_h))
    check("再旋转后仍是同一张原图", ok_len and c.get("sw") == 200 and c.get("sh") == 160,
          "src=%sx%s" % (c.get("sw"), c.get("sh")))

    # ---- Q3b 拉伸再拉回原尺寸：像素必须逐点还原（不吃插值）----
    _, base = run("crisp-base", "extract:20,20,200,160;save")
    _, back = run("crisp-back", "extract:20,20,200,160;scale-layer:300,240;"
                               "scale-layer:200,160;save")
    with Image.open(base) as a_im, Image.open(back) as b_im:
        diff = ImageChops.difference(a_im.convert("RGB"), b_im.convert("RGB"))
        box = diff.getbbox()
        check("拉伸后拉回原尺寸像素不变（不糊）", box is None, "diffbox=%s" % (box,))

    print("PASS: 11-07 复测五项回归")


if __name__ == "__main__":
    main()
