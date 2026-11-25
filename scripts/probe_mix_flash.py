"""交叉淡出「混合区变浓/变淡」诊断探针。

用户 2026-11-25 反馈：放手瞬间会闪一下，「拖动图层和透明区域重合部分不闪，
其它地方混合地方会闪」。

判据推导：放手时 GPU 三明治层与 CPU 落定帧在 90ms 内**同时可见**（opacity
交叉）。若两者内容相同且**不透明**（alpha=255），叠两次仍是自己 → 不闪；
若内容半透明（alpha<255），叠两次会变浓（S over S ≠ S）→ 闪。这正好解释
「透明区（下面没内容，alpha 由图层决定）/ 混合区」的差异。

本探针构造：不透明青底 + 50% 半透明洋红块（Normal，无蒙版 → 走 GPU 三明治），
真实鼠标拖动后松手，在**洋红块覆盖青底**的固定像素上采样颜色轨迹：
  * 拖动中 t<0：稳定色 C
  * 落定稳定 t>1s：同一色 C
  * bug：中间帧颜色偏离 C（变浓 → 更接近纯洋红）

固定像素靠 fixture 布局算出画布坐标 → 用画布左上角锚点换算到窗口坐标。
为避免选框/手柄干扰，采样点取洋红块内部远离边缘处。

usage: python scripts/probe_mix_flash.py [--keep] [--alpha N]
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
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_mix_flash")
PY = sys.executable

CANVAS = (1600, 1000)
BG_TOP, BG_LEFT = 0, 0
BG_W, BG_H = 1600, 1000
MAG_TOP, MAG_LEFT = 200, 200
MAG_W, MAG_H = 800, 600
MOVE = (160, 100)
MAGENTA = (255, 0, 255)
CYAN = (0, 128, 128)

user32 = ctypes.windll.user32
gdi = ctypes.windll.gdi32
PW_RENDERFULLCONTENT = 0x2
WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP = 0x0200, 0x0201, 0x0202
MK_LBUTTON = 0x0001

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and bool(cond)
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


def make_fixture(path: str, alpha: int) -> None:
    """青底不透明（画布基底）+ 半透明洋红块（被拖）。"""
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import make_psd_fixture as mk

    def channel_data(layer, ci):
        w, h = layer["w"], layer["h"]
        if ci == 3:  # alpha —— 带 2 字节 RAW 压缩头（write_psd 约定）
            a = alpha if layer["name"] == "magenta" else 255
            return b"\x00\x00" + bytes([a]) * (w * h)
        return b"\x00\x00" + bytes([layer["color"][ci]]) * (w * h)

    mk.channel_data = channel_data
    # 青底放最后（=最底）。--no-bg 时透明画布，只留半透明洋红块。
    specs = [dict(name="magenta", top=MAG_TOP, left=MAG_LEFT,
                  w=MAG_W, h=MAG_H, color=MAGENTA)]
    if "--no-bg" not in sys.argv:
        specs.append(dict(name="cyan-bg", top=BG_TOP, left=BG_LEFT,
                          w=BG_W, h=BG_H, color=CYAN))
    mk.write_psd(path, CANVAS[0], CANVAS[1], specs)


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
            area = (r.right - r.left) * (r.bottom - r.top)
            if area > 0:
                found.append((area, hwnd))
        return True

    user32.EnumWindows(cb, 0)
    return max(found)[1] if found else None


def client_offset(hwnd):
    pt = wt.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(pt))
    wr = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(wr))
    return pt.x - wr.left, pt.y - wr.top


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


def post(hwnd, msg, x, y, wp=0):
    user32.PostMessageW(hwnd, msg, wp, (int(y) << 16) | (int(x) & 0xFFFF))


def find_canvas_origin(img: Image.Image) -> tuple[int, int] | None:
    """找青色画布左上角：扫描全图找 CYAN 色块的最小 x/y。"""
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    m = (np.abs(r - CYAN[0]) < 12) & (np.abs(g - CYAN[1]) < 12) & (np.abs(b - CYAN[2]) < 12)
    ys, xs = np.where(m)
    if len(xs) < 100:
        return None
    return int(xs.min()), int(ys.min())


def sample(img: Image.Image, x: int, y: int) -> tuple[int, int, int]:
    px = img.convert("RGB").load()
    return px[x, y]


def main() -> int:
    keep = "--keep" in sys.argv
    alpha = 128
    if "--alpha" in sys.argv:
        alpha = int(sys.argv[sys.argv.index("--alpha") + 1])

    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    psd = os.path.join(TMP, "mix.psd")
    make_fixture(psd, alpha)

    try:
        user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    except Exception:
        pass

    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = TMP
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    log = open(os.path.join(TMP, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), psd,
         "--script-after-paint=select-layer:1", "--script-delay=1200"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1)

    frames: list[tuple[str, float, Image.Image]] = []
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
        time.sleep(3.0)
        off = client_offset(hwnd)

        img = grab(hwnd)
        origin = find_canvas_origin(img) if img else None
        check("定位青色画布", origin is not None)
        if origin is None:
            return 1
        ox, oy = origin
        print("canvas origin (window px):", (ox, oy), "client offset:", off)

        # 洋红块在**预览位图**里的位置已知（frame2 = display 尺寸）。
        # 用落定帧的 bbox 直接给出屏幕坐标：预览图铺满 previewImage，
        # previewImage.x/y = Math.round((contentWidth-w)/2)。
        # 更稳的做法：从青色画布的屏幕 bbox 推出缩放，再映射图像坐标。
        a = np.asarray(img.convert("RGB")).astype(np.int16)
        r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
        # 半透明洋红压青底 ≈ (128,64,192)；用「r 明显>青底 且 b 明显>青底」
        mm = (r > 80) & (g < 110) & (b > 140)
        ys, xs = np.where(mm)
        check("找到半透明洋红块", len(xs) > 2000, "n=%d" % len(xs))
        if len(xs) <= 2000:
            return 1
        mx0, mx1, my0, my1 = int(xs.min()), int(xs.max()), int(ys.min()), int(ys.max())
        print("magenta bbox:", (mx0, my0, mx1, my1))
        sx, sy = (mx0 + mx1) // 2, (my0 + my1) // 2
        c_before = sample(img, sx, sy)
        print("before drag sample:", c_before)
        frames.append(("still", 0.0, img))

        # 真实鼠标拖动（用洋红块中心，屏幕坐标）
        cx, cy = sx, sy
        post(hwnd, WM_MOUSEMOVE, cx - off[0], cy - off[1])
        time.sleep(0.05)
        post(hwnd, WM_LBUTTONDOWN, cx - off[0], cy - off[1], MK_LBUTTON)
        time.sleep(0.08)
        steps = 16
        dx_screen = int((mx1 - mx0) * 0.28)   # 约等于图像位移比例
        dy_screen = int((my1 - my0) * 0.18)
        for i in range(1, steps + 1):
            post(hwnd, WM_MOUSEMOVE,
                 (cx - off[0]) + dx_screen * i // steps,
                 (cy - off[1]) + dy_screen * i // steps, MK_LBUTTON)
            time.sleep(0.03)
        time.sleep(0.12)
        img = grab(hwnd)
        if img:
            frames.append(("drag", 0.0, img))

        t_release = time.perf_counter()
        post(hwnd, WM_LBUTTONUP,
             (cx - off[0]) + dx_screen, (cy - off[1]) + dy_screen, 0)
        while time.perf_counter() - t_release < 1.4:
            img = grab(hwnd)
            if img:
                t = time.perf_counter() - t_release
                frames.append(("release", t, img))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    # 采样固定点：拖动后的洋红块中心 = 原中心 + (dx_screen, dy_screen)
    sx2, sy2 = (mx0 + mx1) // 2 + dx_screen, (my0 + my1) // 2 + dy_screen
    print("\nsample point after move:", (sx2, sy2))

    def dist(c1, c2):
        return max(abs(a - b) for a, b in zip(c1, c2))

    vals = []
    for phase, t, img in frames:
        c = sample(img, sx2, sy2)
        vals.append((phase, t, c))
        print("  %-8s %6.3f  %s" % (phase, t, c))

    still = [c for p, _, c in vals if p == "still"]
    drag = [c for p, _, c in vals if p == "drag"]
    rel = [(t, c) for p, t, c in vals if p == "release"]
    base = drag[0] if drag else (still[0] if still else (0, 0, 0))
    settled = rel[-1][1] if rel else base
    check("拖动中采样点有效", dist(base, CYAN) > 20, "base=%s cyan=%s" % (base, CYAN))
    check("落定稳定色与拖动色一致", dist(settled, base) <= 8,
          "drag=%s settled=%s d=%d" % (base, settled, dist(settled, base)))
    # 核心：松手后不应出现明显偏离（变浓 → 更接近纯洋红）
    worst = max((dist(c, base), t, c) for t, c in rel) if rel else (0, 0, base)
    check("松手后混合区无浓度突变（偏差 ≤ 12）", worst[0] <= 12,
          "worst_d=%d @%.3fs color=%s base=%s" % (worst[0], worst[1], worst[2], base))

    if keep:
        for i, (phase, t, im) in enumerate(frames):
            im.save(os.path.join(TMP, "%02d_%s_%d.png" % (i, phase, int(t * 1000))))
        print("\nframes ->", TMP)
    else:
        shutil.rmtree(TMP, ignore_errors=True)

    print("\nRESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
