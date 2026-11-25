"""普通 PNG 场景的放手闪回归：不透明底 PNG + drop-insert 半透明 PNG 层。

对应用户 2026-11-25 报「普通图片、不是 PSD」+「压在别的图上混合区才闪」。
构造与用户截图一致：深色水面底图 + 半透明（带不规则边缘）的水面/船体层，
真实鼠标拖动后松手连拍，检查混合区固定像素不出现浓度突变。

判据与 probe_release_fade 一致：松手后每帧 mass 不超过落定基准 1.25×
（交叉淡出双叠时半透明内容 mass 会尖到 2.5×）。

usage: python scripts/probe_png_mix_flash.py [--keep]
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_png_mix_flash")
PY = sys.executable

user32 = ctypes.windll.user32
gdi = ctypes.windll.gdi32
PW_RENDERFULLCONTENT = 0x2
WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP = 0x0200, 0x0201, 0x0202
MK_LBUTTON = 0x0001
ok = True


def check(name, cond, extra=""):
    global ok
    ok = ok and bool(cond)
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def make_fixtures():
    from PIL import ImageDraw
    bg = Image.new("RGBA", (1200, 800), (18, 72, 84, 255))
    dr = ImageDraw.Draw(bg)
    for y in range(0, 800, 8):
        dr.line([(0, y), (1200, y)], fill=(18 + y // 24, 72 + y // 30, 84 + y // 40, 255), width=3)
    bg_path = os.path.join(TMP, "water_bg.png")
    bg.save(bg_path)

    fg = Image.new("RGBA", (900, 600), (96, 150, 165, 140))
    p = fg.load()
    for y in range(600):
        for x in range(900):
            edge = 720 + int(120 * abs(((y * 7) % 90) / 45 - 1))
            if x > edge:
                p[x, y] = (230, 240, 245, max(0, 255 - (x - edge) * 6))
            else:
                p[x, y] = (96 + ((x * 3 + y) % 20), 150 + (y % 18), 165 + ((x + y) % 15), 130)
    fg_path = os.path.join(TMP, "sail_fg.png")
    fg.save(fg_path)
    return bg_path, fg_path


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                ("biClrImportant", ctypes.c_uint32)]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", ctypes.c_uint32 * 3)]


def find_main_window(pid):
    found = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hwnd, _l):
        wpid = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid and user32.IsWindowVisible(hwnd):
            r = wt.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(r))
            if (r.right - r.left) > 0 and (r.bottom - r.top) > 0:
                found.append(((r.right - r.left) * (r.bottom - r.top), hwnd))
        return True

    user32.EnumWindows(cb, 0)
    return max(found)[1] if found else None


def client_offset(hwnd):
    pt = wt.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(pt))
    wr = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(wr))
    return pt.x - wr.left, pt.y - wr.top


def grab(hwnd):
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


def post(hwnd, msg, x, y, wp=0):
    user32.PostMessageW(hwnd, msg, wp, (int(y) << 16) | (int(x) & 0xFFFF))


def semi_frac(img):
    """半透明浅色层（比底图亮且有蓝调）覆盖面积占比。"""
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    # 底图水面 ≈ (18..51, 72..98, 84..104)：g < 120。半透明层压上后 g 明显升高。
    return float((g > 105).mean()), int((g > 105).sum())


def main():
    keep = "--keep" in sys.argv
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    bg_path, fg_path = make_fixtures()

    try:
        user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    except Exception:
        pass

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = TMP
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    # 打开底图 → 插入半透明层 → 选中新层。drop-insert 异步，给足延迟。
    ops = "drop-insert:" + fg_path.replace("\\", "/")
    log = open(os.path.join(TMP, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), bg_path,
         "--script-after-paint=" + ops, "--script-delay=1500"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1)

    frames = []
    try:
        hwnd = None
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < 15:
            hwnd = find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        check("主窗口出现", hwnd is not None)
        if hwnd is None:
            return 1
        time.sleep(3.5)   # 等 drop-insert 落地
        off = client_offset(hwnd)

        img = grab(hwnd)
        if img:
            frames.append(("still", 0.0, img))
        f0, n0 = semi_frac(img)
        check("半透明层已插入并可见", n0 > 3000, "frac=%.4f n=%d" % (f0, n0))

        # 找半透明层的可拖区域：屏幕上半透明浅色像素的质心
        a = np.asarray(img.convert("RGB")).astype(np.int16)
        g = a[:, :, 1]
        ys, xs = np.where(g > 105)
        if len(xs) < 100:
            check("找到半透明层质心", False)
            return 1
        cx, cy = int(np.median(xs)), int(np.median(ys))
        print("drag center:", (cx, cy), "client offset:", off)

        post(hwnd, WM_MOUSEMOVE, cx - off[0], cy - off[1])
        time.sleep(0.05)
        post(hwnd, WM_LBUTTONDOWN, cx - off[0], cy - off[1], MK_LBUTTON)
        time.sleep(0.08)
        dx, dy = 120, 70
        for i in range(1, 13):
            post(hwnd, WM_MOUSEMOVE,
                 (cx - off[0]) + dx * i // 12, (cy - off[1]) + dy * i // 12, MK_LBUTTON)
            time.sleep(0.03)
        time.sleep(0.12)
        img = grab(hwnd)
        if img:
            frames.append(("drag", 0.0, img))

        t_release = time.perf_counter()
        post(hwnd, WM_LBUTTONUP, (cx - off[0]) + dx, (cy - off[1]) + dy, 0)
        while time.perf_counter() - t_release < 1.2:
            img = grab(hwnd)
            if img:
                frames.append(("release", time.perf_counter() - t_release, img))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    vals = [(p, t, semi_frac(im)[1]) for p, t, im in frames]
    rel = [n for p, t, n in vals if p == "release" and t > 0.4]
    settled = float(np.median(rel)) if rel else 1.0
    peak = max((n for p, t, n in vals if p == "release"), default=0)
    for p, t, n in vals[:6]:
        print("  %-8s %6.3f  n=%d" % (p, t, n))
    check("落定后面积有效", settled > 1000, "settled=%.0f" % settled)
    check("松手后面积不尖峰（无交叉淡出双叠）", peak <= settled * 1.3,
          "peak=%d settled=%.0f ratio=%.2f" % (peak, settled, peak / max(1, settled)))

    if keep:
        for i, (p, t, im) in enumerate(frames):
            im.save(os.path.join(TMP, "%02d_%s_%d.png" % (i, p, int(t * 1000))))
        print("frames ->", TMP)
    else:
        shutil.rmtree(TMP, ignore_errors=True)
    print("\nRESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
