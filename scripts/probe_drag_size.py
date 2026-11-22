"""拖动期图层尺寸回归探针：拖动中画面不许比落定后小一点点。

背景（2026-11-22，用户报「拖动图层时图层画面会缩小一点点，放手后才还原」）：
Tier 1 GPU 拖动期间，屏幕上的活动层来自 `gpuActive` 这张**原始位图**，QML 把
它拉伸到 `gpu_rect`（`offset × disp_scale`、`img 尺寸 × disp_scale`）。任何
一处比例 / 取整算错，拖动期就会看到图层比松手后小一圈。

做法：小画布（480×320，窗口一开就是 100%，`disp_scale` 恰好 1.0）上放一块
纯洋红方块，用 `gpu-hold` 起一次**不落定**的移动手势，连拍屏幕量洋红色块的
包围盒：

  * 拖动中 bbox ≈ 起手前 bbox（±2px）→ PASS；
  * 拖动中 bbox 明显更小 → 复现「缩小一点点」。

usage: python scripts/probe_drag_size.py [--keep]
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time

from PIL import Image, ImageChops

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_drag_size")
PSD = os.path.join(TMP, "drag.psd")

CANVAS = (1200, 800)
LAYER_AT = (120, 90)         # top, left
LAYER_SIZE = (700, 400)      # w, h
MOVE = (100, 60)             # dx, dy（doc px）
MAGENTA = (255, 0, 255)

user32 = ctypes.windll.user32
gdi = ctypes.windll.gdi32
PW_RENDERFULLCONTENT = 0x2

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def make_fixture() -> None:
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import make_psd_fixture as mk

    def channel_data(layer, ci):
        w, h = layer["w"], layer["h"]
        if ci == 3:
            return b"\x00\x00" + bytes([255]) * (w * h)
        return b"\x00\x00" + bytes([layer["color"][ci]]) * (w * h)

    mk.channel_data = channel_data
    mk.write_psd(
        PSD,
        CANVAS[0],
        CANVAS[1],
        [dict(name="magenta", top=LAYER_AT[0], left=LAYER_AT[1],
              w=LAYER_SIZE[0], h=LAYER_SIZE[1], color=MAGENTA)],
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


def magenta_mask(img: Image.Image) -> Image.Image:
    r, g, b = img.split()
    rm = r.point(lambda v: 255 if v > 195 else 0)
    gm = g.point(lambda v: 255 if v < 60 else 0)
    bm = b.point(lambda v: 255 if v > 195 else 0)
    return ImageChops.multiply(ImageChops.multiply(rm, gm), bm)


def bbox(img: Image.Image):
    m = magenta_mask(img)
    return m.getbbox()          # (x0, y0, x1, y1) 或 None


def main() -> int:
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    make_fixture()

    try:
        user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    except Exception:
        pass

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = TMP
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    script = "select-layer:1;gpu-hold:%d,%d" % MOVE
    log = open(os.path.join(TMP, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), PSD,
         "--script-after-paint=" + script, "--script-delay=900"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1,
    )

    t0 = time.perf_counter()
    frames: list[tuple[float, tuple | None]] = []
    shots: list[tuple[float, Image.Image]] = []
    try:
        hwnd = None
        while time.perf_counter() - t0 < 15:
            hwnd = find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        check("主窗口已出现", hwnd is not None)
        if hwnd is None:
            return 1

        deadline = time.perf_counter() + 3.2
        while time.perf_counter() < deadline:
            img = grab(hwnd)
            if img is None:
                continue
            t = time.perf_counter() - t0
            frames.append((t, bbox(img)))
            if "--keep" in sys.argv:
                shots.append((t, img))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    log_text = open(os.path.join(TMP, "run.log"), encoding="utf-8", errors="replace").read()
    check("手势已起（未落定）", '"event":"gpu-hold","began":true' in log_text)
    check("走了 GPU 三明治快路径",
          '"event":"gpu-begin"' in log_text or '"event":"gpu-begin-set"' in log_text)

    good = [(t, b) for t, b in frames if b is not None]
    check("拍到足够多的帧", len(good) >= 20, "%d 帧 / 跨度 %.2fs" % (
        len(good), good[-1][0] if good else 0))
    if not good:
        print("RESULT FAIL")
        return 1

    def size(b):
        return (b[2] - b[0], b[3] - b[1])

    # 起手段（脚本执行前，画面静止在原始位置）与拖动态（最后一帧位置已明显偏移）
    first = size(good[0][1])
    moved = [b for _, b in good if abs(b[0] - good[0][1][0]) > 4]
    check("拖动确实发生了", len(moved) > 0, "起手 x=%d" % good[0][1][0])
    if not moved:
        print("counts:", " ".join("%.2f:%s" % (t, b) for t, b in good[:12]))
        print("RESULT FAIL")
        return 1

    drag = size(moved[-1])
    check("拖动中图层尺寸与起手一致（宽）", abs(drag[0] - first[0]) <= 2,
          "起手 %dx%d → 拖动中 %dx%d" % (first[0], first[1], drag[0], drag[1]))
    check("拖动中图层尺寸与起手一致（高）", abs(drag[1] - first[1]) <= 2,
          "起手 %dx%d → 拖动中 %dx%d" % (first[0], first[1], drag[0], drag[1]))

    print("frames:", " ".join("%.2f:%s" % (t, b) for t, b in good[::max(1, len(good) // 14)]))
    if "--keep" in sys.argv:
        for i, (t, im) in enumerate(shots):
            im.save(os.path.join(TMP, "f%02d_%d.png" % (i, int(t * 1000))))
        print("frames ->", TMP)
    else:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
