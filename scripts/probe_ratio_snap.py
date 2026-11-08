#!/usr/bin/env python
"""等比（Shift）缩放 + 智能吸附的回归探针。

用户报的现象：按住 Shift 拖角点做等比拉伸时，只要附近有可吸附的边，
图层就被拉成非等比（宽高比变了）。根因是吸附修正只加在被拖那一轴的
尺寸上，另一轴不动。

脚本侧用 `--script=scale-solve:role,dx,dy,mods,disp` 直接跑求解器（含
锁比例 / 吸附），断言：
  1) Shift + 吸附命中：命中那根边的位置精确等于参考线，且 w/h 保持起始比例；
  2) 参考线只保留生效的那一轴（另一轴被比例带偏，不该再画线）；
  3) 不按 Shift 时行为不变（单轴吸附，高度不动）；
  4) 没有可吸附目标时等比也不受影响。

用法：python scripts/probe_ratio_snap.py
"""
import json
import os
import subprocess
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
FIXTURE = os.path.join(ROOT, "probe_ratio_snap.png")
LOG = os.path.join(ROOT, "probe_ratio_snap.log")

# 元素（被拉伸的图层）：400..690 × 250..450 → 290×200，比例 1.45
ELEM = (400, 250, 290, 200)
# 参考块：左缘在 x=700，提供一条竖向参考线（元素右缘 698 距离 2）
BLOCK = (700, 100, 60, 500)

SHIFT = 0x02000000


def fixture():
    img = Image.new("RGBA", (900, 700), (255, 255, 255, 255))
    d = ImageDraw.Draw(img)
    d.rectangle([BLOCK[0], BLOCK[1], BLOCK[0] + BLOCK[2] - 1, BLOCK[1] + BLOCK[3] - 1],
                fill=(40, 40, 40, 255))
    d.rectangle([ELEM[0], ELEM[1], ELEM[0] + ELEM[2] - 1, ELEM[1] + ELEM[3] - 1],
                fill=(200, 60, 60, 255))
    img.save(FIXTURE)


def solve(role, dx, dy, mods=0):
    """起一个实例跑一次求解，返回 (start, solved, guide_axes)。"""
    script = ";".join([
        # 先抠参考块，再抠元素 —— 元素（最后抠出的那个）自动成为选中图层，
        # `transform_solve_scale` 解的就是它。
        # ★两次 extract 之间必须 `select-layer:-1`：选区会被裁到「当前选中
        #   图层」里，第一层选着时第二块区域根本不在裁切范围内 → 第二次
        #   extract 静默失败（只剩一层，解算的是参考块本身）。
        "extract:%d,%d,%d,%d" % BLOCK, "sel-none", "select-layer:-1",
        "extract:%d,%d,%d,%d" % ELEM, "sel-none",
        "scale-solve:%s,%g,%g,%d,1" % (role, dx, dy, mods),
    ])
    args = [EXE, FIXTURE, "--no-restore-tool", "--script=" + script, "--auto-quit"]
    env = dict(os.environ)
    env["PATH"] = os.environ.get("QT_BIN", "/d/Qt/6.8.3/msvc2022_64/bin") + ";" + env.get("PATH", "")
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    r = subprocess.run(args, capture_output=True, text=True, errors="replace", timeout=120,
                       env=env, cwd=ROOT)
    out = (r.stdout or "") + (r.stderr or "")
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(out)
    res = None
    for line in out.splitlines():
        if '"event":"scale-solve"' not in line:
            continue
        at = line.find('{"phase"')
        if at < 0:
            continue
        try:
            obj, _ = json.JSONDecoder().raw_decode(line[at:])
        except ValueError:
            continue
        res = obj
    return res


def ratios_ok(solved, start):
    ar0 = start[2] / start[3]
    ar1 = solved["w"] / solved["h"]
    return abs(ar1 - ar0) < 1e-6, ar0, ar1


def main():
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fixture()
    if os.path.exists(LOG):
        os.remove(LOG)

    fails = []

    def check(name, cond, detail):
        print("  %-26s %s  %s" % (name, "PASS" if cond else "FAIL", detail))
        if not cond:
            fails.append(name)

    # ---- 1) Shift + 命中竖向参考线 -----------------------------------
    r = solve("se", 8, 25, SHIFT)
    if not r:
        print("FAIL 没有 scale-solve 输出（见 %s）" % LOG)
        return 1
    s, start = r["solved"], (r["sx"], r["sy"], r["sw"], r["sh"])
    ok, ar0, ar1 = ratios_ok(s, start)
    check("等比被吸附拉坏", ok, "ar %.6f → %.6f（start %s）" % (ar0, ar1, start))
    check("命中参考线 x=700", abs(s["x"] + s["w"] - 700.0) < 1e-6,
          "右缘 %.3f" % (s["x"] + s["w"]))
    guides = r.get("guides", [])
    x_lines = [g for g in guides if g.get("v")]
    y_lines = [g for g in guides if not g.get("v")]
    check("参考线落在 700", any(abs(float(g.get("p", 0)) - 700.0) < 0.5 for g in x_lines),
          "guides=%s" % guides)
    check("不画无效轴参考线", len(y_lines) == 0, "y 线 %d 条" % len(y_lines))

    # ---- 2) 不按 Shift：行为不变（单轴吸附，高度只跟光标）--------------
    r = solve("se", 8, 25, 0)
    s, start = r["solved"], (r["sx"], r["sy"], r["sw"], r["sh"])
    check("自由缩放仍命中", abs(s["x"] + s["w"] - 700.0) < 1e-6, "右缘 %.3f" % (s["x"] + s["w"]))
    check("自由缩放高度不动", abs(s["h"] - (start[3] + 25)) < 1e-6, "h=%.3f" % s["h"])

    # ---- 3) 没有可吸附目标：等比 + 吸附互不干扰 ----------------------
    r = solve("se", 80, 20, SHIFT)
    s, start = r["solved"], (r["sx"], r["sy"], r["sw"], r["sh"])
    ok, ar0, ar1 = ratios_ok(s, start)
    check("无目标时等比不变", ok, "ar %.6f → %.6f" % (ar0, ar1))
    check("无目标时不画线", len(r.get("guides", [])) == 0, "guides=%s" % r.get("guides", []))

    print()
    print("RESULT", "ALL PASS" if not fails else "FAIL")
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
