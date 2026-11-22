"""拖动期像素一致性探针：GPU 拖动帧的图层内容必须和静止帧**逐像素一致**。

比「量直径」严一个数量级：给一次整数位移的拖动手势（位移在 25% / 50% /
100% 下都恰好是整数显示像素），把 GPU 帧按位移挪回来，和 CPU 静止帧做逐像素
比对 —— 半像素平移、边缘 AA 丢失、整图双线性重采样都会暴露在覆盖率差上。

为什么分两次运行：`zoom:N` 必须放在 `--script-after-paint`（`--script` 在文档
加载前执行，zoom 会被当成 100%），而它和手势同一时刻生效，混在一起就分不清
「定档造成的画面对不上」和「GPU 拖动造成的画面对不上」。所以：
  A 次跑 `zoom:N;select-layer:1`  → CPU 静止帧（真值）
  B 次跑 `zoom:N;select-layer:1;gpu-hold:dx,dy` → GPU 拖动帧
两次用自己的包围盒对齐，位置差只当作断言（应等于位移），不影响像素比对。

用纯洋红的圆形图层（UI 里没有洋红），圆对边缘最敏感。

usage: python scripts/probe_drag_pixel.py [--zoom 25,50,100] [--move 100,60] [--keep]
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe_drag_zoom as base

TMP = os.path.join(os.environ.get("TEMP", "."), "probe_drag_pixel")

# 与 demo_batch.psd 的「蓝色圆」同几何（900×600 画布、220px 圆在 (150,190)）
base.CANVAS = (900, 600)
base.DIAM = 220
base.LAYER_AT = (190, 150)     # top, left
MOVE = (100, 60)               # 25%→(25,15) 50%→(50,30) 100%→(100,60)，全是整数
SETTLE = 2.2


def coverage(img: np.ndarray) -> np.ndarray:
    """洋红覆盖率 0..1（满覆盖归一）。

    ★必须掐掉拖动时画的**对齐参考线**（实测色 (224,64,251)）：它也是「红蓝高、
    绿低」，用 `min(r-g, b-g)` 会把它当成洋红算进来，包围盒被拉到整块画布。
    圆是纯洋红 (255,0,255)，绿通道为 0，所以按 g ≤ 32 一刀切。
    """
    r, g, b = img[:, :, 0], img[:, :, 1], img[:, :, 2]
    w = np.minimum(r - g, b - g).astype(np.float32)
    w[g > 32] = 0.0
    np.clip(w, 0.0, None, out=w)
    return np.clip(w / 255.0, 0.0, 1.0)


def capture(pct: int, gesture: bool, move: tuple[int, int]) -> np.ndarray | None:
    tmp = os.path.join(TMP, "z%d%s" % (pct, "g" if gesture else "c"))
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    psd = os.path.join(tmp, "circle.psd")
    base.make_fixture(psd)
    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    script = "zoom:%d;select-layer:1" % pct
    if gesture:
        script += ";gpu-hold:%d,%d" % move
    log = open(os.path.join(tmp, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(base.ROOT, "target", "debug", "imageedit.exe"), psd,
         "--script-after-paint=" + script, "--script-delay=1100"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1,
    )
    last = None
    count = 0
    t0 = time.perf_counter()
    try:
        hwnd = None
        while time.perf_counter() - t0 < 15:
            hwnd = base.find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        if hwnd is None:
            base.check("[%d%%%s] 主窗口已出现" % (pct, "/拖" if gesture else "/静"), False)
            return None
        deadline = time.perf_counter() + SETTLE + 1.4
        while time.perf_counter() < deadline:
            im = base.grab(hwnd)
            if im is not None and time.perf_counter() - t0 > SETTLE:
                last = np.asarray(im.convert("RGB")).astype(np.int16)
                count += 1
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    text = open(os.path.join(tmp, "run.log"), encoding="utf-8", errors="replace").read()
    if gesture:
        base.check("[%d%%] 手势已起" % pct, '"event":"gpu-hold","began":true' in text)
        base.check("[%d%%] 走了 GPU 快路径" % pct, '"event":"gpu-begin"' in text)
    base.check("[%d%%%s] 拍到帧" % (pct, "/拖" if gesture else "/静"), count >= 5,
               "%d 帧" % count)
    return last


def run(pct: int, move: tuple[int, int], keep: bool):
    cpu_img = capture(pct, False, move)
    gpu_img = capture(pct, True, move)
    if cpu_img is None or gpu_img is None:
        return None

    c_cpu = coverage(cpu_img)
    c_gpu = coverage(gpu_img)
    mask = c_cpu > 0.15
    if not mask.any():
        base.check("[%d%%] 静止帧里有洋红圆" % pct, False)
        return None
    ys, xs = np.nonzero(mask)
    want = base.DIAM * pct / 100.0
    got = float(xs.max() - xs.min() + 1)
    base.check("[%d%%] 静止帧已经是目标缩放" % pct, abs(got - want) <= 3.0,
               "圆宽 %.0fpx（期望 %.0f）" % (got, want))

    m2 = c_gpu > 0.15
    if not m2.any():
        base.check("[%d%%] 拖动帧里有洋红圆" % pct, False)
        return None
    ys2, xs2 = np.nonzero(m2)
    dx, dy = move
    want_off = (int(round(dx * pct / 100.0)), int(round(dy * pct / 100.0)))
    got_off = (int(xs2.min() - xs.min()), int(ys2.min() - ys.min()))
    print("      位移实测 %s（期望 %s，两次运行窗口尺寸可能不同）"
          % (got_off, want_off))

    # 两次运行各用自己的包围盒对齐（窗口尺寸/位置会有差异，绝对坐标不可比），
    # 两侧各留 3px（覆盖 AA 半径）。内容一致时两张区块应逐像素相同。
    y0, y1 = max(0, int(ys.min()) - 3), int(ys.max()) + 4
    x0, x1 = max(0, int(xs.min()) - 3), int(xs.max()) + 4
    gy, gx = max(0, int(ys2.min()) - 3), max(0, int(xs2.min()) - 3)
    ay0, ay1 = max(0, int(ys.min()) - 3), int(ys.max()) + 4
    ax0, ax1 = max(0, int(xs.min()) - 3), int(xs.max()) + 4
    by0, by1 = max(0, int(ys2.min()) - 3), int(ys2.max()) + 4
    bx0, bx1 = max(0, int(xs2.min()) - 3), int(xs2.max()) + 4
    h = min(ay1 - ay0, by1 - by0)
    w = min(ax1 - ax0, bx1 - bx0)
    a = c_cpu[ay0:ay0 + h, ax0:ax0 + w]
    b = c_gpu[by0:by0 + h, bx0:bx0 + w]
    if a.shape != b.shape:
        base.check("[%d%%] 裁剪区域一致" % pct, False, "%s vs %s" % (a.shape, b.shape))
        return None

    diff = np.abs(a - b)
    d_mad = float(diff.mean())
    big = int((diff > 0.15).sum())
    mass_ratio = float(b.sum() / max(1e-6, a.sum()))
    print("      区块 %dx%d  MAD=%.4f  max=%.2f  差异像素(>0.15)=%d/%d  面积比=%.4f"
          % (w, h, d_mad, float(diff.max()), big, w * h, mass_ratio))
    base.check("[%d%%] 拖动帧与静止帧逐像素一致" % pct,
               d_mad < 0.02 and big < 40 and abs(mass_ratio - 1.0) < 0.01,
               "MAD=%.4f big=%d 面积%+.2f%%" % (d_mad, big, (mass_ratio - 1) * 100))

    if keep:
        out = os.path.join(TMP, "diff_%d.png" % pct)
        trio = np.concatenate([a, b, diff], axis=1)
        Image.fromarray(np.clip(trio * 255, 0, 255).astype(np.uint8)).save(out)
        print("      diff ->", out)
    return d_mad, big, mass_ratio


def main() -> int:
    zooms = [25, 50, 100]
    if "--zoom" in sys.argv:
        zooms = [int(v) for v in sys.argv[sys.argv.index("--zoom") + 1].split(",")]
    move = MOVE
    if "--move" in sys.argv:
        a, b = sys.argv[sys.argv.index("--move") + 1].split(",")
        move = (int(a), int(b))
    keep = "--keep" in sys.argv
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    try:
        base.user32.SetThreadDpiAwarenessContext(-4)
    except Exception:
        pass

    for pct in zooms:
        print("---- zoom %d%% ----" % pct)
        run(pct, move, keep)
    print("RESULT", "PASS" if base.ok else "FAIL")
    return 0 if base.ok else 1


if __name__ == "__main__":
    sys.exit(main())
