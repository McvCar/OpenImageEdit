"""GPU 三明治纹理回归探针：拖**非顶层**图层时，above 纹理绝不能含活动层。

背景（2026-11-22）：gpu_begin 合成 above（上层栈）时只隐藏了 [..index]，
活动层自身仍可见 —— 活动层被按原位置烘进 above 纹理，拖动时 QML 按
「下 → 活动 → 上」叠三层，上层纹理里的残影 + 移动中的活动层 = 用户看到
「两个图层」。probe_gpu_overlay.py 抓不到它：那里 gpu-gesture 拖的是
paste 出来的**顶层**（above 为空），而 bug 只在拖非顶层且上方有可见层时出现。

本探针做**纹理级**断言（gpu_begin 落盘的 s/a/o 三张 bmp），比屏幕截图准：
  * static（-s-）：只有基底 → 无黄块、无蓝块；
  * active（-a-）：活动层本体 → 必须含黄块；
  * above （-o-）：只有上方层 → 必须含蓝块、**绝不含黄块**。
另附屏幕 == 最后一帧的兜底断言（沿用 probe_gpu_overlay 的对齐 MAD）。

usage: python scripts/probe_gpu_sandwich.py [--keep]
"""
from __future__ import annotations

import glob
import os
import re
import shutil
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageStat

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = sys.executable
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_gpu_sandwich")
IMG = os.path.join(TMP, "card.png")
SHOT = os.path.join(TMP, "screen.png")

# 两层浮动层：黄块 L1（index 1）、蓝块 L2（index 2，顶层）。
# ★第二次 cut 前必须 select-layer:-1 取消选中 —— pick_target 里 selected==0
#   会打到位图层 0（locked，被拒），只有 -1 才回落到 Base（恒不锁）。
# select-layer:1 选下面的黄块 → gpu-gesture 拖它（走三明治路径）。
SCRIPT = (
    "cut:60,60,90,80;paste;"
    "select-layer:-1;cut:230,200,80,60;paste;"
    "select-layer:1;gpu-gesture:40,30"
)

YELLOW = (214, 186, 74)
BLUE = (60, 90, 200)

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def mad(a: Image.Image, b: Image.Image) -> float:
    return sum(ImageStat.Stat(ImageChops.difference(a.convert("RGB"), b.convert("RGB"))).mean) / 3


def color_frac(im: Image.Image, ref: tuple[int, int, int], tol: int = 45) -> float:
    """匹配 ref 颜色（±tol）的像素占比（0..1）。"""
    px = im.convert("RGB").load()
    w, h = im.size
    hit = 0
    for y in range(0, h, 2):          # 隔行采样足够区分 4% 级差异
        for x in range(0, w, 2):
            r, g, b = px[x, y][:3]
            if abs(r - ref[0]) <= tol and abs(g - ref[1]) <= tol and abs(b - ref[2]) <= tol:
                hit += 1
    total = ((w + 1) // 2) * ((h + 1) // 2)
    return hit / max(1, total)


def make_card(path: str) -> None:
    from PIL import ImageDraw

    im = Image.new("RGB", (360, 318), (48, 20, 24))
    d = ImageDraw.Draw(im)
    for y in range(318):
        d.line([(0, y), (360, y)], fill=(60 + y // 8, 22 + y // 12, 26 + y // 10))
    # 黄块（将被切成 L1）：亮色 + 内嵌白/黑，切出后颜色特征极强
    d.rectangle([60, 60, 150, 140], fill=YELLOW)
    d.rectangle([70, 76, 140, 108], fill=(250, 240, 220))
    d.rectangle([78, 84, 132, 100], fill=(30, 30, 30))
    # 蓝块（将被切成 L2，顶层）：与黄块颜色距离足够远
    d.rectangle([230, 200, 310, 260], fill=BLUE)
    d.rectangle([238, 208, 302, 252], fill=(150, 180, 255))
    im.save(path)


def latest_bmp(kind: str) -> str | None:
    """取 tag 最大的 imageedit-gpu-*-{kind}-*.bmp。"""
    files = glob.glob(os.path.join(TMP, f"imageedit-gpu-*-{kind}-*.bmp"))
    if not files:
        return None
    return max(files, key=lambda f: int(re.search(r"-(\d+)\.bmp$", f).group(1)))


def canvas_rect(win: Image.Image) -> tuple[int, int, int, int]:
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


def main() -> int:
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    make_card(IMG)

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = TMP
    qt_root = env.get("IMAGEEDIT_QT_ROOT", r"D:\Qt\6.8.3\msvc2022_64")
    env["PATH"] = os.path.join(qt_root, "bin") + ";" + env.get("PATH", "")
    log_path = os.path.join(TMP, "run.log")
    log = open(log_path, "w", encoding="utf-8")
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

    log_text = open(log_path, encoding="utf-8", errors="replace").read()

    # ---- 1) 日志：确实走了三明治 GPU 路径，拖的是 index 1 ----
    begins = re.findall(r'\{"phase":"ui","event":"gpu-begin","index":(\d+),"full":(\w+)', log_text)
    check("gpu-begin 落在 index 1（非顶层）", ("1", "false") in begins,
          "gpu-begin=%s" % begins)
    check("手势完成", '"event":"gpu-gesture","began":true,"landed":true' in log_text)

    # ---- 2) 纹理级断言：三明治契约 ----
    s = latest_bmp("s")
    a = latest_bmp("a")
    o = latest_bmp("o")
    check("三张纹理都已落盘", None not in (s, a, o))
    if None not in (s, a, o):
        im_s, im_a, im_o = (Image.open(p) for p in (s, a, o))
        y_s = color_frac(im_s, YELLOW)
        b_s = color_frac(im_s, BLUE)
        y_a = color_frac(im_a, YELLOW)
        b_o = color_frac(im_o, BLUE)
        y_o = color_frac(im_o, YELLOW)
        check("static 只有基底（无黄/蓝）", y_s < 0.005 and b_s < 0.005,
              "yellow=%.3f%% blue=%.3f%%" % (y_s * 100, b_s * 100))
        check("active 是黄块本体", y_a > 0.01, "yellow=%.3f%%" % (y_a * 100))
        check("above 含蓝块（上方层在）", b_o > 0.005, "blue=%.3f%%" % (b_o * 100))
        check("above 不含黄块（★本次回归：活动层残影）", y_o < 0.005,
              "yellow=%.3f%%" % (y_o * 100))

    # ---- 3) 兜底：松手后屏幕 == 最后一帧 ----
    frames = sorted(glob.glob(os.path.join(TMP, "imageedit-preview-0-*.bmp")))
    if frames and os.path.exists(SHOT):
        win = Image.open(SHOT).convert("RGB")
        x0, y0, x1, y1 = canvas_rect(win)
        check("画布已定位", (y1 - y0) > 200, "bbox=%s" % ((x0, y0, x1, y1),))
        px = win.load()
        mid = (y0 + y1) // 2
        xs = [x for x in range(300, win.width - 40) if sum(px[x, mid][:3]) > 90]
        left = xs[0] if xs else x0
        ref_rgba = Image.open(frames[-1]).convert("RGBA")
        # ★只比不透明区域：cut 留下的洞在帧里是 alpha=0，屏幕上显示棋盘格
        #   （透明指示）是**正确**行为 —— 全图比对会把这一合法差异算成 FAIL。
        mask = ref_rgba.getchannel("A").point(lambda v: 255 if v > 128 else 0)
        ref_rgb = ref_rgba.convert("RGB")
        black = Image.new("RGB", ref_rgb.size, (0, 0, 0))

        def small(im: Image.Image) -> Image.Image:
            # 45×40 BOX 缩小摊平选框虚线 / 手柄 / 尺寸徽标（沿用 overlay 探针）
            return im.resize((45, 40), Image.BOX)

        best = 1e9
        r = small(black)
        for dy in range(-12, 13, 2):
            for dx in range(-12, 13, 2):
                x, y = left + dx, y0 + dy
                if x < 0 or y < 0 or x + 360 > win.width or y + 318 > win.height:
                    continue
                d = ImageChops.difference(win.crop((x, y, x + 360, y + 318)).convert("RGB"), ref_rgb)
                m = small(Image.composite(d, black, mask))
                best = min(best, sum(ImageStat.Stat(m).mean) / 3)
        check("松手后屏幕 == 最后一帧（不透明区域）", best < 4.0, "mad=%.2f" % best)
    else:
        check("有预览帧与屏幕截图", False)

    if "--keep" not in sys.argv:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
