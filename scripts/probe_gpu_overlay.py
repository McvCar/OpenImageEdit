"""GPU 覆盖层回归探针：移动手势之后再缩放落定，**屏幕上的画布必须等于最后一帧**。

背景（2026-11-08）：`gpuLayer`（Tier 1 两张纹理）的 `visible` 曾经是
`gpuLive || 淡出计时器在跑`，计时器被 stop 之后 `visible` 恒为 true，于是
「静态背景 + 活动层停在旧矩形」永久压在 CPU 预览之上：拖动时看着像图层被
背景挡住、松手画面也不变大，再移动一下才恢复。后端帧一直是对的，只有屏幕
是旧的 —— 所以必须拿**屏幕像素**断言，只看 BMP 抓不到这个 bug。

usage: python scripts/probe_gpu_overlay.py [--keep]
"""
from __future__ import annotations

import glob
import os
import shutil
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageStat

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = sys.executable
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_gpu_overlay")
IMG = os.path.join(TMP, "card.png")
SHOT = os.path.join(TMP, "screen.png")

# 用户报的那种操作序列：选区 → 切出图层 → 移动（走 Tier 1）→ 拖手柄放大落定
SCRIPT = (
    "sel-rect:60,60,150,140;cut:60,60,90,80;paste;"
    "gpu-gesture:-30,65;scale-layer:164,193"
)

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def mad(a: Image.Image, b: Image.Image) -> float:
    return sum(ImageStat.Stat(ImageChops.difference(a.convert("RGB"), b.convert("RGB"))).mean) / 3


def canvas_rect(win: Image.Image) -> tuple[int, int, int, int]:
    """在窗口截图里定位画布：非背景色区域的亮像素包围盒（画布高宽比固定 318）。"""
    px = win.load()
    W, H = win.size
    minx, miny, maxx, maxy = W, H, -1, -1
    for y in range(120, H - 80):
        for x in range(40, W - 40):
            r, g, b = px[x, y][:3]
            if r > 60 or g > 60 or b > 60:
                minx = min(minx, x)
                maxx = max(maxx, x)
                miny = min(miny, y)
                maxy = max(maxy, y)
    return minx, miny, maxx, maxy


def make_card(path: str) -> None:
    """画一张合成测试图：深色渐变 + 细网格 + 一块高通量色块（接下来被切出来放大）。

    用合成图而不是用户素材：仓库里不留用户内容，而且色块区域纹理密度高，
    「放大后是否上屏」用像素差就能判出来（原图 vs 放大帧差异远大于噪声）。
    """
    from PIL import ImageDraw

    im = Image.new("RGB", (360, 318), (48, 20, 24))
    d = ImageDraw.Draw(im)
    for y in range(318):
        d.line([(0, y), (360, y)], fill=(60 + y // 8, 22 + y // 12, 26 + y // 10))
    for x in range(0, 360, 7):
        d.line([(x, 0), (x, 318)], fill=(86, 30, 34))
    # 被剪切出来的内容块：亮色 + 斜纹 + 粗字，放大后与原图差异极大
    d.rectangle([60, 60, 150, 140], fill=(214, 186, 74))
    for i in range(-80, 120, 12):
        d.line([(60 + i, 140), (60 + i + 80, 60)], fill=(120, 40, 30), width=4)
    d.rectangle([70, 76, 140, 108], fill=(250, 240, 220))
    d.rectangle([78, 84, 132, 100], fill=(30, 30, 30))
    im.save(path)


def main() -> int:
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    make_card(IMG)

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = TMP
    qt_root = env.get("IMAGEEDIT_QT_ROOT", r"D:\Qt\6.8.3\msvc2022_64")
    env["PATH"] = os.path.join(qt_root, "bin") + ";" + env.get("PATH", "")
    log = open(os.path.join(TMP, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), IMG, "--script=" + SCRIPT],
        stdout=log,
        stderr=subprocess.STDOUT,
        env=env,
    )
    try:
        time.sleep(6)
        subprocess.run([PY, os.path.join(ROOT, "scripts", "grab_screen.py"),
                        str(proc.pid), SHOT], check=True, capture_output=True)
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    win = Image.open(SHOT).convert("RGB")
    x0, y0, x1, y1 = canvas_rect(win)
    check("画布已定位（高度 318）", (y1 - y0 + 1) == 318, "bbox=%s" % ((x0, y0, x1, y1),))
    # 画布左右边界：手柄会外扩，取每行最亮的连续 360 宽
    px = win.load()
    mid = (y0 + y1) // 2
    xs = [x for x in range(300, win.width - 40) if sum(px[x, mid][:3]) > 90]
    left = xs[0] if xs else x0
    shot = win.crop((left, y0, left + 360, y0 + 318))

    frames = sorted(glob.glob(os.path.join(TMP, "imageedit-preview-0-*.bmp")))
    check("后端有预览帧", len(frames) >= 5, "%d 帧" % len(frames))

    def small(im: Image.Image) -> Image.Image:
        return im.convert("RGB").resize((45, 40), Image.BOX)

    def aligned_mad(ref: Image.Image) -> float:
        """在 ±12px 内对齐再比：窗口画布起点随标题栏/滚动条会有几像素漂移。

        缩到 45×40 是为了摊平选框虚线 / 手柄 / 尺寸徽标 —— 那些是画在画布
        之上的控件，逐像素比会把它们算成内容差异。
        """
        best = 1e9
        r = small(ref)
        for dy in range(-12, 13, 2):
            for dx in range(-12, 13, 2):
                x = left + dx
                y = y0 + dy
                if x < 0 or y < 0 or x + 360 > win.width or y + 318 > win.height:
                    continue
                best = min(best, mad(small(win.crop((x, y, x + 360, y + 318))), r))
        return best

    d_last = min(aligned_mad(Image.open(f)) for f in frames[-3:])
    d_src = aligned_mad(Image.open(IMG))
    check("屏幕画布 == 最后一帧（GPU 覆盖层没有残留）", d_last < 4.0, "mad=%.2f" % d_last)
    check("屏幕画布 != 原始图（确实落在新尺寸上）", d_last + 3.0 < d_src,
          "mad_frame=%.2f mad_src=%.2f" % (d_last, d_src))

    # 被放大的图层内容必须真的在屏幕上：取放大框中一块（排除选框线）
    patch = (40, 70, 150, 150)
    p_shot = shot.crop(patch)
    p_src = Image.open(IMG).crop(patch)
    check("放大框内不是原图内容", mad(p_shot, p_src) > 6.0, "mad=%.2f" % mad(p_shot, p_src))

    if "--keep" not in sys.argv:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
