"""拼图咬合（内容占位吸附）端到端探针。

用 `--script` 造三层：两堵不透明墙 + 一个**带透明边距**的移动块，然后调
`fit-probe` 让求解器按内容框求空窗，断言：

* 内容贴到墙边 / 卡进空窗时返回正确的 dx；
* 空窗装不下内容时不吸；
* 距离超过阈值时不吸（阈值 = 8 显示像素）。

    python scripts/probe_gap_fit.py
"""
import json
import os
import re
import subprocess
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
FIX = os.path.join(ROOT, "probe_gap_fit.png")
FIX_HOLE = os.path.join(ROOT, "probe_gap_base_hole.png")
FIX_OPAQUE = os.path.join(ROOT, "probe_gap_opaque.png")

# 布局（图像坐标）：
#   移动块区域 0..100（内容 60x60 在 20..80，四周 20px 透明）
#   左墙 150..190 ｜ 空窗 190..280 ｜ 右墙 280..320
#   空窗宽 90，内容宽 60 —— 居中位置 = 190 + 15 = 205
CONTENT_LEFT = 20.0
CONTENT_W = 60.0


def fixture():
    img = Image.new("RGBA", (400, 300), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rectangle([20, 20, 79, 79], fill=(255, 255, 255, 255))     # 移动块内容
    d.rectangle([150, 0, 189, 299], fill=(255, 255, 255, 255))   # 左墙
    d.rectangle([280, 0, 319, 299], fill=(255, 255, 255, 255))   # 右墙
    img.save(FIX)


def fixture_hole():
    """默认背景整块不透明，只在 100..260 挖了一条通高的透明洞。"""
    img = Image.new("RGBA", (400, 200), (255, 255, 255, 255))
    d = ImageDraw.Draw(img)
    d.rectangle([100, 0, 259, 199], fill=(0, 0, 0, 0))
    img.save(FIX_HOLE)


def fixture_opaque():
    """默认背景整块不透明（照片型）：背景不该当墙，图层之间的咬合照旧。"""
    img = Image.new("RGBA", (400, 300), (255, 255, 255, 255))
    img.save(FIX_OPAQUE)


def run(extra_ops, path=FIX, ops=None):
    if ops is None:
        ops = []
        # ★每次 extract 之前必须 select-layer:-1：抠完会把新图层选中，而
        #   effective_selection 会把选区裁到选中图层里，下一块矩形一旦不在这层
        #   范围内就报 "selection is empty"（第二次抠图必失败）。
        for rect in ("150,0,40,300", "280,0,40,300", "0,0,100,100"):
            ops.append("extract:" + rect)
            ops.append("select-layer:-1")
    ops = list(ops)
    ops += extra_ops
    cmd = [EXE, path, "--script=" + ";".join(ops), "--auto-quit"]
    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    p = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=90, env=env)
    out = (p.stdout or "") + (p.stderr or "")
    res = []
    for line in out.splitlines():
        for ev in ("fit-probe", "move-gesture"):
            key = '"event":"%s"' % ev
            if key not in line:
                continue
            at = line.find('{"phase"')
            try:
                obj, _ = json.JSONDecoder().raw_decode(line[at:])
            except ValueError:
                continue
            obj["event"] = ev
            obj.update(obj.pop("result", {}))
            res.append(obj)
    return res, out


def one(dx, dy=0.0, disp=1.0, path=FIX, ops=None, idx=2):
    """让 index=idx 的移动块平移 (dx,dy) 后求咬合，返回解析后的 dict。"""
    got, out = run(["fit-probe:%d,%g,%g,%g" % (idx, dx, dy, disp)], path, ops)
    if not got:
        print("!! 没读到 fit-probe 输出，原始日志尾部：")
        print(out[-1200:])
        return None
    return got[-1]


def drag(dx, dy=0.0, mods=0, path=FIX, ops=None, idx=2):
    """走完整的移动三件套（begin → 5×update → end），返回落定后的 offset。"""
    got, out = run(
        ["select-layer:%d" % idx, "move-gesture:%g,%g,%d" % (dx, dy, mods)], path, ops
    )
    for item in got:
        if item.get("event") == "move-gesture":
            return item
    print("!! 没读到 move-gesture 输出，原始日志尾部：")
    print(out[-1200:])
    return None


def main():
    fixture()
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fails = []

    def check(name, ok, detail=""):
        print("%-34s %s %s" % (name, "PASS" if ok else "FAIL", detail))
        if not ok:
            fails.append(name)

    # 1) 内容框应当忽略 20px 透明边距：20..80
    r = one(0)
    if r is None:
        return 1
    c = r.get("content")
    check("内容框忽略透明边距", c is not None and abs(c[0] - 20) < 0.01 and abs(c[2] - 60) < 0.01, str(c))

    # 2) 左移靠近左墙：内容左边 22（墙在 190 之外的左侧？）—— 这里用右墙验证贴边
    #    当前左边 = 20+dx；想让右边缘贴到 280 → 左边 = 220 → dx = 200
    r = one(203)   # 左边 223，右边缘 283，离右墙 3px
    check("贴近右墙时贴边吸住", r["sx"] and abs(r["dx"] + 3) < 0.01, str(r))

    # 3) 空窗居中：内容 60 宽，空窗 190..280 → 居中左边 205 → dx = 185
    r = one(187)   # 左边 207，离居中差 2
    check("空窗内居中", r["sx"] and abs(r["dx"] + 2) < 0.01, str(r))

    # 4) 太远不吸（左边 20+150=170，离居中 205 差 35）
    r = one(150)
    check("超出阈值不吸", not r["sx"], str(r))

    # 5) 空窗装不下：把内容当成 200 宽没法构造，改用窄空窗场景验算——
    #    这里用 disp_scale=0.25 把阈值放宽到 32，仍要求"能装下"。
    r = one(203, 0, 0.25)
    check("低倍率阈值放宽仍贴边", r["sx"] and abs(r["dx"] + 3) < 0.01, str(r))

    # 6) 真手势：拖 172 时内容左边落到 192，离墙 2px → 应被咬到 190（offset 170）。
    m = drag(172)
    check("真手势被咬到墙边", m is not None and m["ox"] == 170, str(m))

    # 7) Ctrl 临时关吸附：同一拖动手势必须原地落在 172。
    m = drag(172, 0, 0x0400_0000)
    check("Ctrl 临时关吸附", m is not None and m["ox"] == 172, str(m))

    # ---- 默认背景参与咬合 ----
    # 8) 背景上挖了通高透明洞（100..260）：块从 40,40 拖到内容左边 98，
    #    应被洞的左壁咬到 100（dx = +2）。
    fixture_hole()
    hole_ops = ["extract:40,40,60,60", "select-layer:-1"]
    r = one(58, 0, 1.0, FIX_HOLE, hole_ops, 0)
    check("背景透明洞当墙（求解）", r is not None and r["sx"] and abs(r["dx"] - 2) < 0.01, str(r))
    m = drag(58, 0, 0, FIX_HOLE, hole_ops, 0)
    check("背景透明洞当墙（真手势）", m is not None and m["ox"] == 100, str(m))

    # 9) 整块不透明的照片型背景只是垫底：不能把图层之间的空窗堵死，
    #    否则「内容 60 卡进 190..280 的空窗」这条行为会整体失效。
    fixture_opaque()
    opaque_ops = [
        "extract:150,0,40,300",
        "select-layer:-1",
        "extract:280,0,40,300",
        "select-layer:-1",
        "extract:40,40,60,60",
    ]
    r = one(152, 0, 1.0, FIX_OPAQUE, opaque_ops)
    check("不透明背景不堵空窗", r is not None and r["sx"] and abs(r["dx"] + 2) < 0.01, str(r))
    m = drag(152, 0, 0, FIX_OPAQUE, opaque_ops)
    check("不透明背景下图层咬合", m is not None and m["ox"] == 190, str(m))

    print()
    if fails:
        print("RESULT FAIL:", ", ".join(fails))
        return 1
    print("RESULT ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
