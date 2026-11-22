"""拖动期尺寸回归（降采样档）：25% / 50% / 100% 三档，用**圆形**测。

背景（2026-11-23，用户报「缩放窗口 50% 25% 时拖动图层依旧会缩放」）：
上一版（commit eedb947）在 100% / 87% 上已经对齐，但降采样倍数大的档位还有
两处可疑：

  * `gpu_physical()` 的 `too_big` 闸门（源图面积 > 12 × 目标面积）会直接
    **跳过预缩**，把整张原图交给 Qt 做双线性 4× 降采样 —— 双线性只采 2×2
    邻域，圆弧边缘被欠采样，视觉直径缩水；
  * 预缩用的是图层自身包围盒，而 CPU 落定帧是在**画布网格**上做面积平均，
    两者的取整基准不同。

为什么用圆：矩形边缘只差半列像素，阈值 bbox 看不出来；圆的视觉直径对边缘
反走样极其敏感，用「面积积分」量等效直径能把 1px 环的丢失放大成可读数字。

做法：画布 1200×800 上放一个正圆图层（560 直径），`zoom:N` 定档后起一次
不落定的拖动手势，连拍，按 bbox 聚类分成「CPU 静止段」和「GPU 拖动段」，
比较两段的面积积分等效直径。

usage: python scripts/probe_drag_zoom.py [--zoom 25,50,100] [--keep]
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import math
import os
import shutil
import subprocess
import sys
import time

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_drag_zoom")

CANVAS = (1200, 800)
DIAM = 560                   # 圆的直径（doc px）
LAYER_AT = (120, 90)         # top, left（图层包围盒与圆外接，正方形）
MOVE = (60, 40)              # dx, dy（doc px）—— 够大，能把两段 bbox 分开
MAGENTA = (255, 0, 255)

user32 = ctypes.windll.user32
gdi = ctypes.windll.gdi32
PW_RENDERFULLCONTENT = 0x2

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def make_fixture(path: str) -> None:
    """一个正圆图层：alpha 按像素到圆心的距离切，边缘留 1px 抗锯齿带。"""
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import make_psd_fixture as mk

    size = DIAM
    r = DIAM / 2.0
    cx = cy = r
    # 预计算每个像素的覆盖率（1px 抗锯齿带，用像素中心到圆周的有符号距离）
    cov = bytearray(size * size)
    for y in range(size):
        dy = (y + 0.5) - cy
        for x in range(size):
            dx = (x + 0.5) - cx
            d = math.hypot(dx, dy)
            a = r - d + 0.5          # >1 全内，<0 全外
            if a <= 0.0:
                continue
            cov[y * size + x] = 255 if a >= 1.0 else int(round(a * 255.0))

    def channel_data(layer, ci):
        w, h = layer["w"], layer["h"]
        if ci == 3:
            return b"\x00\x00" + bytes(cov)
        # 圆外 RGB 也填满（alpha 为 0，不影响结果，但便于验证没被意外裁掉）
        return b"\x00\x00" + bytes([layer["color"][ci]]) * (w * h)

    mk.channel_data = channel_data
    mk.write_psd(
        path, CANVAS[0], CANVAS[1],
        [dict(name="circle", top=LAYER_AT[0], left=LAYER_AT[1],
              w=size, h=size, color=MAGENTA)],
    )


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                ("biClrImportant", ctypes.c_uint32)]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", ctypes.c_uint32 * 3)]


def find_main_window(pid: int):
    found = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hwnd, _l):
        wpid = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid and user32.IsWindowVisible(hwnd):
            r = wt.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(r))
            if (r.right - r.left) * (r.bottom - r.top) > 0:
                found.append(((r.right - r.left) * (r.bottom - r.top), hwnd))
        return True

    user32.EnumWindows(cb, 0)
    return max(found)[1] if found else None


def grab(hwnd) -> Image.Image | None:
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.right - r.left, r.bottom - r.top
    if w <= 0 or h <= 0:
        return None
    hdc = user32.GetWindowDC(hwnd)
    mem = gdi.CreateCompatibleDC(hdc)
    bmp = gdi.CreateCompatibleBitmap(hdc, w, h)
    gdi.SelectObject(mem, bmp)
    user32.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)
    bmi = BITMAPINFO()
    bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.bmiHeader.biWidth = w
    bmi.bmiHeader.biHeight = -h
    bmi.bmiHeader.biPlanes = 1
    bmi.bmiHeader.biBitCount = 32
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bmi), 0)
    img = Image.frombuffer("RGBA", (w, h), buf.raw, "raw", "BGRA", 0, 1).convert("RGB")
    gdi.DeleteObject(bmp)
    gdi.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    return img


def measure(img: Image.Image):
    """洋红覆盖率积分：mass = 覆盖面积(px²)，等效直径按正圆反解。

    用面积而不是 bbox：圆的 bbox 只受最外侧那一列影响，还会被 PrintWindow 的
    整格量化吃掉；面积对每一条边缘像素的覆盖率都敏感，1px 的 AA 环丢失约等于
    直径缩水 2/直径。
    """
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    w = np.minimum(r - g, b - g).astype(np.float32) / 255.0
    np.clip(w, 0.0, 1.0, out=w)
    mass = float(w.sum())
    if mass < 500.0:
        return None
    ys, xs = np.nonzero(w > 0.5)
    if len(xs) == 0:
        return None
    return dict(
        mass=mass,
        dia=2.0 * math.sqrt(mass / math.pi),
        box=(int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1),
    )


SETTLE = 2.2          # 脚本 1.1s 执行完；之后画面才稳定，只取这段时间的帧


def run_once(pct: int, gesture: bool, keep: bool) -> list[tuple[float, dict, Image.Image]]:
    """跑一次实例，返回 (t, 量测, 图) 列表。

    `gesture=False` 是 CPU 基准（只定档不拖），`True` 是 GPU 拖动。
    两段都只取 t > SETTLE 的帧 —— 脚本执行前画面还停在「适应窗口」的档位上，
    把那些帧混进来会把静止段量成 0.8675 倍，看不出真正的差。
    """
    tag = "z%d%s" % (pct, "g" if gesture else "c")
    tmp = os.path.join(TMP, tag)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    psd = os.path.join(tmp, "circle.psd")
    make_fixture(psd)

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    script = "zoom:%d;select-layer:1" % pct
    if gesture:
        script += ";gpu-hold:%d,%d" % MOVE
    log = open(os.path.join(tmp, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), psd,
         "--script-after-paint=" + script, "--script-delay=1100"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1,
    )

    shots: list[tuple[float, dict, Image.Image]] = []
    t0 = time.perf_counter()
    try:
        hwnd = None
        while time.perf_counter() - t0 < 15:
            hwnd = find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        if hwnd is None:
            check("[%d%%%s] 主窗口已出现" % (pct, "/拖" if gesture else "/静"), False)
            return []
        deadline = time.perf_counter() + SETTLE + 1.6
        while time.perf_counter() < deadline:
            img = grab(hwnd)
            if img is not None:
                t = time.perf_counter() - t0
                mm = measure(img)
                if mm and t > SETTLE:
                    shots.append((t, mm, img))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    text = open(os.path.join(tmp, "run.log"), encoding="utf-8", errors="replace").read()
    if gesture:
        check("[%d%%] 手势已起（未落定）" % pct,
              '"event":"gpu-hold","began":true' in text)
        check("[%d%%] 走了 GPU 三明治快路径" % pct, '"event":"gpu-begin"' in text)
        import re
        m = re.search(r'\{"phase":"ui","event":"gpu-begin".*?\}', text)
        if m:
            print("      gpu-begin:", m.group(0))
    if keep:
        for i, (t, _mm, im) in enumerate(shots[::max(1, len(shots) // 10)]):
            im.save(os.path.join(tmp, "f%02d_%d.png" % (i, int(t * 1000))))
        print("      frames ->", tmp)
    return shots


def run_zoom(pct: int, keep: bool) -> dict | None:
    still = run_once(pct, False, keep)
    check("[%d%%] CPU 基准帧足够" % pct, len(still) >= 5, "%d 帧" % len(still))
    moved = run_once(pct, True, keep)
    check("[%d%%] GPU 拖动帧足够" % pct, len(moved) >= 5, "%d 帧" % len(moved))
    if len(still) < 5 or len(moved) < 5:
        return None

    s_dia = float(np.median([r[1]["dia"] for r in still]))
    m_dia = float(np.median([r[1]["dia"] for r in moved]))
    s_box = still[len(still) // 2][1]["box"]
    m_box = moved[len(moved) // 2][1]["box"]
    delta = (m_dia - s_dia) / s_dia * 100.0
    print("      CPU(静止) 等效直径 %.2fpx（bbox %dx%d）→ GPU(拖动) %.2fpx"
          "（bbox %dx%d）  %+.2f%%"
          % (s_dia, s_box[2] - s_box[0], s_box[3] - s_box[1],
             m_dia, m_box[2] - m_box[0], m_box[3] - m_box[1], delta))
    # 容差 0.35%：560 直径在 25% 下是 140px，0.35% ≈ 0.5px 直径
    check("[%d%%] 拖动期圆的等效直径等于静止段" % pct, abs(delta) <= 0.35,
          "%+.2f%%" % delta)
    return dict(pct=pct, still=s_dia, moved=m_dia, delta=delta)


def main() -> int:
    zooms = [25, 50, 100]
    if "--zoom" in sys.argv:
        zooms = [int(v) for v in sys.argv[sys.argv.index("--zoom") + 1].split(",")]
    keep = "--keep" in sys.argv
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    try:
        user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    except Exception:
        pass

    results = []
    for pct in zooms:
        print("---- zoom %d%% ----" % pct)
        r = run_zoom(pct, keep)
        if r:
            results.append(r)
    print("---- summary ----")
    for r in results:
        print("  %3d%%  静止 %.2f → 拖动 %.2f  %+.2f%%"
              % (r["pct"], r["still"], r["moved"], r["delta"]))
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
