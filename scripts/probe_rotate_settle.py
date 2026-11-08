"""旋柄旋转：**拖动中的最后一帧**与**松手落定后的帧**必须落在同一处。

背景（2026-11-08）：用户报「旋转中的画面和放手的画面有些不一致」。两条路径
的几何本来就不一样：

  * 拖动帧：`composite_live_rotate_adjusted` —— 把 snapshot 转到**固定的正方形**
    （对角线长）再贴到 `round(offset*sx)`；
  * 落定帧：`LayerTransform::render_compatible` —— 转到**紧致 AABB**，贴到
    `round(center - w/2)`。

两边的取整互相独立，还会因为外接框宽高奇偶变化而相位不同，于是松手瞬间内容
可能平移 0.5~1px（外加一次重采样）。本探针把两帧 BMP 对齐比较，输出残差与
最佳整数位移：**位移必须为 (0,0)**，残差只允许来自低质/高质采样器差异。

usage: python scripts/probe_rotate_settle.py [--keep] [--img 素材]
"""
from __future__ import annotations

import glob
import os
import shutil
import subprocess
import sys

from PIL import Image, ImageChops, ImageStat

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_rotate_settle")

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def mad(a: Image.Image, b: Image.Image) -> float:
    d = ImageChops.difference(a.convert("RGB"), b.convert("RGB"))
    return sum(ImageStat.Stat(d).mean) / 3


def make_fixture(path: str) -> None:
    """360×318 合成图：细网格 + 高对比色块，旋转后角点位置可分辨到 1px。"""
    from PIL import ImageDraw

    im = Image.new("RGB", (360, 318), (40, 46, 58))
    d = ImageDraw.Draw(im)
    for y in range(0, 318, 9):
        d.line([(0, y), (360, y)], fill=(58, 66, 82))
    for x in range(0, 360, 11):
        d.line([(x, 0), (x, 318)], fill=(66, 74, 92))
    d.rectangle([70, 60, 260, 210], fill=(208, 96, 62))
    for i in range(-140, 200, 26):
        d.line([(70 + i, 210), (70 + i + 130, 60)], fill=(24, 26, 32), width=7)
    d.ellipse([96, 84, 176, 164], fill=(240, 226, 180))
    d.rectangle([110, 98, 162, 150], fill=(30, 34, 44))
    im.save(path)


def run_frames(img: str, ops: str, tag: str) -> list[str]:
    """跑一个隔离实例（独立 TEMP），返回它产出的预览帧路径（按时间排序）。"""
    tmp = os.path.join(TMP, tag)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    qt_root = env.get("IMAGEEDIT_QT_ROOT", r"D:\Qt\6.8.3\msvc2022_64")
    env["PATH"] = os.path.join(qt_root, "bin") + ";" + env.get("PATH", "")
    with open(os.path.join(tmp, "run.log"), "w", encoding="utf-8") as log:
        subprocess.run(
            [EXE, img, "--script=" + ops, "--auto-quit"],
            stdout=log, stderr=subprocess.STDOUT, env=env, timeout=180,
        )
    return sorted(
        glob.glob(os.path.join(tmp, "imageedit-preview-*.bmp")),
        key=os.path.getmtime,
    )


def best_shift(a: Image.Image, b: Image.Image, radius: int = 4):
    """在 ±radius 内找让 b 最像 a 的整数位移；先缩到 1/4 摊平重采样噪声。"""
    sw, sh = a.width // 4, a.height // 4
    small_a = a.convert("RGB").resize((sw, sh), Image.BOX)
    best = (1e9, 0, 0)
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            shifted = ImageChops.offset(b.convert("RGB"), dx, dy)
            m = mad(small_a, shifted.resize((sw, sh), Image.BOX))
            if m < best[0]:
                best = (m, dx, dy)
    return best


def main() -> int:
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    args = sys.argv[1:]
    img = args[args.index("--img") + 1] if "--img" in args else os.path.join(TMP, "card.png")
    if "--img" not in args:
        make_fixture(img)
    # 缩放比很关键：预览帧是在「预览尺寸」上取整的，落定是在画布尺寸上取整的，
    # 两者换算到屏幕的误差随 zoom 放大。默认 100%，用 --zoom 覆盖。
    zoom = int(args[args.index("--zoom") + 1]) if "--zoom" in args else 100

    # 同一个角度（23°）分别走「拖住不放」和「拖完松手」，比较两边的画面。
    # extract 出来的图层就是即将被旋转的那一层，选区已消耗，无虚线干扰。
    zoom_op = "zoom:%d;" % zoom
    base = "extract:60,50,240,200;"
    # 三条路径都要对齐：裸图层 / 先拉伸（走 render_compatible 的 Lanczos 分支）/
    # 已经转过一次（预览快照是未插值原图，落定却按 params 重新采样）。
    scenes = [
        ("裸图层", base),
        ("先拉伸", base + "scale-layer:300,220;"),
        ("二次旋转", base + "rotate-handle:0,11,1;"),
        ("拉伸后二次", base + "scale-layer:300,220;rotate-handle:0,11,1;"),
        # ★半像素中心：`transform-params` 用「外接框左上角 + 尺寸」，w=213 为
        # 奇数 → center = 100+106.5 = 206.5。这是 11-08 用户日志里的真实状态
        # （center.y=175.5）。拖动帧正方形很大，`center - nw/2` 变成**负数**，
        # 一旦用 `f64::round()`（.5 远离零）就会比落定少 1px。
        ("半像素中心", base + "transform-params:100,100,213,157,0,0;"),
        ("半像素中心+二次", base + "transform-params:100,100,213,157,0,0;rotate-handle:0,11,1;"),
    ]
    for name, pre in scenes:
        hold = run_frames(img, zoom_op + pre + "rotate-hold:0,23", "hold_" + name)
        settle = run_frames(img, zoom_op + pre + "rotate-handle:0,23,1", "settle_" + name)
        if not hold or not settle:
            check("[%s] 两帧都落盘" % name, False, "%d/%d" % (len(hold), len(settle)))
            continue
        A = Image.open(hold[-1]).convert("RGB")
        B = Image.open(settle[-1]).convert("RGB")
        m0 = mad(A, B)
        mbest, dx, dy = best_shift(A, B)
        check("[%s] 松手没有让画面整体平移" % name, (dx, dy) == (0, 0),
              "最佳位移=(%d,%d) mad %.2f → %.2f" % (dx, dy, m0, mbest))
        check("[%s] 两帧内容基本重合" % name, m0 < 12.0, "mad=%.2f" % m0)

    if "--keep" not in args:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
