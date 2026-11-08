"""「透明区域当图层」对齐探针。

咬合（`fit_into_gaps`）只在内容**装得下**空窗时生效；本探针验的是另一条
路径：把被内容围住的透明区域（洞）当成虚拟图层，用它的外接框产出六条
边 / 中心线，让**比洞大**的元素也能把某条边压在洞壁上、或者居中对齐。

布局（1600x900，图像坐标）：

    不透明块 600..1400 x 100..800
      洞里挖空 900..1100 x 300..500（200x200）
    元素（红色方块）260x260 放在 0..260 —— 比洞大，咬合求解器必然拒绝

    python scripts/probe_holes_snap.py
"""
import json
import os
import subprocess

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
FIX = os.path.join(ROOT, "probe_holes_snap.png")

HOLE = (900.0, 300.0, 200.0, 200.0)
ELEM = 260.0  # 元素边长，> 洞的 200 → 一定塞不进去


def fixture():
    img = Image.new("RGBA", (1600, 900), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rectangle([600, 100, 1399, 799], fill=(255, 255, 255, 255))       # 照片块
    d.rectangle([900, 300, 1099, 499], fill=(0, 0, 0, 0))               # 洞
    d.rectangle([0, 0, 259, 259], fill=(200, 30, 40, 255))              # 元素
    img.save(FIX)


def run(ops):
    cmd = [EXE, FIX, "--script=" + ";".join(ops), "--auto-quit"]
    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    p = subprocess.run(cmd, capture_output=True, text=True, errors="replace",
                       timeout=120, env=env)
    out = (p.stdout or "") + (p.stderr or "")
    res = []
    for line in out.splitlines():
        for ev in ("holes", "snap-move", "move-gesture"):
            if '"event":"%s"' % ev not in line:
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


def main():
    fixture()
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fails = []

    def check(name, ok, detail=""):
        print("%-38s %s %s" % (name, "PASS" if ok else "FAIL", detail))
        if not ok:
            fails.append(name)

    def rects(item):
        return [tuple(round(v, 1) for v in r) for r in (item.get("rects") or [])]

    # ---- 1) 洞的识别 ----
    got, out = run([
        "extract:0,0,260,260", "sel-none",
        "holes:-1",   # 默认背景
        "holes:0",    # 元素层（整块不透明）
    ])
    if not got:
        print("!! 没读到输出，日志尾部：")
        print(out[-1200:])
        return 1
    backgrounds = [r for r in got if r["event"] == "holes"]
    check("背景洞里被识别成对齐基准", bool(backgrounds) and rects(backgrounds[0]) == [HOLE],
          str(backgrounds[0] if backgrounds else None))
    check("整块不透明的图层没有假洞",
          len(backgrounds) > 1 and rects(backgrounds[1]) == [],
          str(backgrounds[1] if len(backgrounds) > 1 else None))

    # ---- 2) 元素比洞大，仍能吸到洞壁 / 洞中心 ----
    # 元素左边 896（离洞左壁 900 差 4）→ dx = +4
    # 元素左边 874（中心 1004，洞中心 1000）→ dx = -4
    # 元素左边 760（离最近的 1000/800 都 > 8）→ 不吸
    got, out = run([
        "extract:0,0,260,260", "sel-none",
        "snap-move:0,896,0",
        "snap-move:0,874,0",
        "snap-move:0,760,0",
    ])
    snaps = [r for r in got if r["event"] == "snap-move"]
    if len(snaps) < 3:
        print("!! snap-move 数量不足：", len(snaps))
        print(out[-1200:])
        return 1
    check("元素边压到洞壁（贴左壁）", abs(snaps[0]["dx"] - 4) < 0.01, str(snaps[0]))
    check("元素中心对齐洞中心", abs(snaps[1]["dx"] + 4) < 0.01, str(snaps[1]))
    check("离洞太远不吸", abs(snaps[2]["dx"]) < 0.01, str(snaps[2]))
    check("洞壁参考线出现在 x=900",
          any(abs(g[1] - 900) < 0.01 for g in snaps[0].get("guides", [])),
          str(snaps[0].get("guides")))

    # ---- 3) 洞在**图层**里（不是背景）同样当对齐基准 ----
    got, out = run([
        "extract:850,250,400,300", "select-layer:-1",
        "extract:0,0,260,260", "sel-none",
        "holes:0",
        "snap-move:1,896,0",
    ])
    layer_holes = [r for r in got if r["event"] == "holes"]
    check("图层里的洞也被识别",
          bool(layer_holes) and rects(layer_holes[0]) == [HOLE],
          str(layer_holes[0] if layer_holes else None))
    snaps = [r for r in got if r["event"] == "snap-move"]
    check("吸到图层里的洞壁",
          bool(snaps) and abs(snaps[0]["dx"] - 4) < 0.01,
          str(snaps[0] if snaps else None))

    # ---- 4) 真手势：整条变换管线下也要咬到洞壁（不是只有探针求解器） ----
    for mods, want, name in ((0, 900.0, "真手势吸到洞壁"), (0x0400_0000, 896.0, "Ctrl 临时关吸附")):
        got, out = run([
            "extract:0,0,260,260", "sel-none", "select-layer:0",
            "move-gesture:896,0,%d" % mods,
        ])
        moves = [r for r in got if r["event"] == "move-gesture"]
        check(name, bool(moves) and abs(moves[0]["ox"] - want) < 0.01,
              str(moves[0] if moves else None))

    # ---- 5) 坐标换算：带 offset 的图层，洞框跟着偏移 ----
    # 注意不能先 select-layer:-1（那是取消选中），transform-params 写的是
    # 当前选中层；extract 之后新层已经自动选中。
    got, out = run([
        "extract:850,250,400,300",
        "transform-params:900,350,400,300,0,0,4",  # 挪 (+50,+100)
        "holes:0",
    ])
    moved = [r for r in got if r["event"] == "holes"]
    check("洞框随图层位移",
          bool(moved) and rects(moved[0]) == [(950.0, 400.0, 200.0, 200.0)],
          str(moved[0] if moved else None))

    print()
    if fails:
        print("RESULT FAIL:", ", ".join(fails))
        return 1
    print("RESULT ALL PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
