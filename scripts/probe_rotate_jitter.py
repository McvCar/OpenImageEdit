"""旋柄**持续旋转**时，图层内容不得每帧来回抖（亚像素级跳动）。

背景（2026-11-08，用户报「持续旋转图层时图层在来回抖动，要放大 200 多倍才
看清楚」）：

  拖动帧原先用当前角度的 AABB 反推内容中心：
      center = round(cx - W/2) + W/2
  AABB 宽高随角度逐度变化、奇偶交替，于是 center 在 `.0` 与 `.5` 之间来回
  跳，整层每帧位移 1px。现在 `expanded_size` 把外接框尺寸向上取偶，中心恒等于
  `round(cx)`，与角度无关。

测法：一次 begin + 连续多次 update（`rotate-drag`），逐帧取预览 BMP 里
「高饱和红块」的质心。正方形绕自身中心旋转质心不变，所以质心序列的任何
波动都是位置抖动，不是内容变化。

usage: python scripts/probe_rotate_jitter.py [--keep]
"""
from __future__ import annotations

import glob
import os
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_rotate_jitter")

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def make_fixture(path: str) -> None:
    """500×380：深色网格底 + 一块纯红正方形，**红块中心 = 图层中心**。

    `extract:x,y,w,h` 是「左上角 + 尺寸」，本脚本切 (120,90) 起、340×270 的
    区域 → 图层中心 = (120+170, 90+135) = (290,225)。红块必须画在这个中心上，
    否则旋转时它绕图层中心公转，质心本来就会动，测不出抖动。
    """
    im = Image.new("RGB", (500, 380), (38, 44, 56))
    d = ImageDraw.Draw(im)
    for y in range(0, 380, 7):
        d.line([(0, y), (500, y)], fill=(52, 60, 74))
    for x in range(0, 500, 9):
        d.line([(x, 0), (x, 380)], fill=(60, 68, 84))
    d.rectangle([230, 165, 350, 285], fill=(212, 58, 48))
    im.save(path)


def make_big_fixture(path: str) -> None:
    """3000×2200 大图：验证大图层（1:1 全尺寸预览）同样不抖。

    `extract:120,90,2800,2000` → 图层中心 = (1520,1090)，红块画在那里。
    注意大图预览帧单张 26MB，临时文件只保留 3 帧，所以 --big 的门槛是 3 帧。
    """
    im = Image.new("RGB", (3000, 2200), (38, 44, 56))
    d = ImageDraw.Draw(im)
    for y in range(0, 2200, 23):
        d.line([(0, y), (3000, y)], fill=(52, 60, 74))
    for x in range(0, 3000, 31):
        d.line([(x, 0), (x, 2200)], fill=(60, 68, 84))
    d.rectangle([1320, 890, 1720, 1290], fill=(212, 58, 48))
    im.save(path)


def make_half_fixture(path: str) -> None:
    """半像素中心：`extract:120,90,341,271` → 图层中心 (290.5, 225.5)。

    红块必须精确画在这个中心上（121×121，覆盖 230..350 / 165..285），否则
    旋转时它会绕中心公转，质心本来就会动。
    """
    im = Image.new("RGB", (500, 380), (38, 44, 56))
    d = ImageDraw.Draw(im)
    for y in range(0, 380, 7):
        d.line([(0, y), (500, y)], fill=(52, 60, 74))
    for x in range(0, 500, 9):
        d.line([(x, 0), (x, 380)], fill=(60, 68, 84))
    d.rectangle([230, 165, 350, 285], fill=(212, 58, 48))
    im.save(path)


def run_frames(img: str, ops: str) -> tuple[list[str], str]:
    tmp = os.path.join(TMP, "drag")
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    qt_root = env.get("IMAGEEDIT_QT_ROOT", r"D:\Qt\6.8.3\msvc2022_64")
    env["PATH"] = os.path.join(qt_root, "bin") + ";" + env.get("PATH", "")
    log_path = os.path.join(tmp, "run.log")
    with open(log_path, "w", encoding="utf-8") as log:
        subprocess.run(
            [EXE, img, "--script=" + ops, "--auto-quit"],
            stdout=log, stderr=subprocess.STDOUT, env=env, timeout=180,
        )
    frames = sorted(
        glob.glob(os.path.join(tmp, "imageedit-preview-*.bmp")),
        key=os.path.getmtime,
    )
    return frames, log_path


def red_centroid(path: str) -> tuple[float, float, int]:
    im = Image.open(path).convert("RGB")
    px = im.load()
    w, h = im.size
    sx = sy = n = 0
    for y in range(h):
        for x in range(w):
            r, g, b = px[x, y]
            if r > 150 and g < 120 and b < 120:
                sx += x
                sy += y
                n += 1
    if n == 0:
        return (0.0, 0.0, 0)
    return (sx / n, sy / n, n)


def reversals(seq: list[float], eps: float = 0.05) -> int:
    """相邻差分的方向翻转次数（真抖动会来回翻，单调旋转不会）。"""
    signs = []
    for a, b in zip(seq, seq[1:]):
        d = b - a
        if abs(d) > eps:
            signs.append(1 if d > 0 else -1)
    return sum(1 for a, b in zip(signs, signs[1:]) if a != b)


def main() -> int:
    args = sys.argv[1:]
    if "--keep" not in args:
        shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    img = os.path.join(TMP, "fixture.png")
    make_fixture(img)

    # 一次 begin + 9 次 update：模拟按住旋柄一路转过去（每步 2.6°）。
    ops = "extract:120,90,340,270;rotate-drag:0,2.6,5.2,7.8,10.4,13.0,15.6,18.2,20.8,23.4"
    if "--big" in args:
        # 大图层：预览走降采样（sx<1），验证缩放路径同样不抖。
        img = os.path.join(TMP, "big.png")
        make_big_fixture(img)
        ops = "extract:120,90,2800,2000;rotate-drag:0,2.6,5.2,7.8,10.4,13.0,15.6,18.2,20.8,23.4"
    if "--half" in args:
        # 半像素中心（341/271 都是奇数 → center 落在 .5 上）：拖动帧必须先用
        # `round_center` 把中心量化到落定的整像素栅格，否则每帧都带 .5。
        img = os.path.join(TMP, "half.png")
        make_half_fixture(img)
        ops = "extract:120,90,341,271;rotate-drag:0,2.6,5.2,7.8,10.4,13.0,15.6,18.2,20.8,23.4"
    frames, log = run_frames(img, ops)
    # 大图预览帧单张就有 20~30MB，临时文件保留策略只留 3 帧（见 session.rs
    # 的按体积保留），所以门槛跟着降。
    need = 3 if "--big" in args else 6
    check("至少落盘 %d 帧拖动预览" % need, len(frames) >= need, "%d 帧" % len(frames))
    if len(frames) < need:
        return 1

    cs = [red_centroid(f) for f in frames]
    check("每帧都能找到图层内容", all(c[2] > 500 for c in cs),
          "面积 %s" % [c[2] for c in cs])
    # 前两帧是 extract 之后的静止画面（还没开始转），它们与旋转帧之间会差
    # 半个像素（1:1 贴图 vs 重采样），不属于抖动。只量「开始旋转之后」的帧。
    rot = cs[2:] if len(cs) >= 6 else cs
    xs = [c[0] for c in rot]
    ys = [c[1] for c in rot]
    spread_x = max(xs) - min(xs)
    spread_y = max(ys) - min(ys)
    check("内容中心左右不抖动", spread_x <= 0.45,
          "极差 %.2fpx  序列 %s" % (spread_x, ["%.1f" % v for v in xs]))
    check("内容中心上下不抖动", spread_y <= 0.45,
          "极差 %.2fpx  序列 %s" % (spread_y, ["%.1f" % v for v in ys]))
    rx, ry = reversals(xs), reversals(ys)
    check("中心单调不来回摆", rx <= 1 and ry <= 1, "翻转 x=%d y=%d" % (rx, ry))

    if "--keep" not in args:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL", "| log:", log)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
