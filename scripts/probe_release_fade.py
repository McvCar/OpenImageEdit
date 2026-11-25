"""真实鼠标落定瞬间「图层变透明」诊断探针。

与 probe_settle_flash.py 的区别：那个走 --script=gpu-gesture（同步脚本路径，
transform_begin/update/end 一个事件回合跑完），测不到真实 MouseArea 的松手
时序。这里用 PostMessage 投真实鼠标事件 + PrintWindow 高频连拍**松手前后**，
量每一个采样帧里洋红图层的「质量」(alpha 加权积分)。

现象假设（用户 2026-11-25 报）：
  * 拖动中 mass ≈ C（GPU active 纹理满不透明）
  * 落定稳定后 mass ≈ C
  * bug 帧：mass 掉到 ~0.5C 或更低但非 0 = 「图层变透明/半透明闪一下」
  * 更糟：mass → 0 = 整层消失（CPU 预览空窗）
  * 另一类：mass → 2C = 新旧两份同时可见

用透明画布 + 洋红实心块（Normal、无蒙版 → 走 GPU 三明治快路径）。

usage: python scripts/probe_release_fade.py [--keep] [--zoom 100]
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
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_release_fade")
PY = sys.executable

CANVAS = (1600, 1000)
LAYER_AT = (200, 200)          # (top, left)
LAYER_SIZE = (800, 600)
MOVE = (200, 120)              # 显示像素位移（100% 档）
MAGENTA = (255, 0, 255)

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


# ---------------------------------------------------------------- fixture ----
def make_fixture(path: str, layers: int = 1, circle: bool = False,
                 opacity: int = 255) -> None:
    """layers=1 单层透明背景；layers=3 底(青)+中(洋红,被拖)+顶(黄)。
    circle=True 时把洋红块换成带抗锯齿 alpha 边缘的圆（测边缘交接）。"""
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import make_psd_fixture as mk
    import math as _math

    cov_cache = {}

    def channel_data(layer, ci):
        w, h = layer["w"], layer["h"]
        if ci == 3:  # alpha channel (write_psd uses range(4))
            if circle and layer["name"] == "magenta":
                if "cov" not in cov_cache:
                    r = w / 2.0
                    cov = bytearray(w * h)
                    for y in range(h):
                        dy = (y + 0.5) - r
                        for x in range(w):
                            d = _math.hypot((x + 0.5) - r, dy)
                            a = r - d + 0.5
                            if a <= 0:
                                continue
                            cov[y * w + x] = 255 if a >= 1 else int(round(a * 255))
                    cov_cache["cov"] = bytes(cov)
                return b"\x00\x00" + cov_cache["cov"]
            return b"\x00\x00" + bytes([255]) * (w * h)
        return b"\x00\x00" + bytes([layer["color"][ci]]) * (w * h)

    mk.channel_data = channel_data
    if layers == 1:
        specs = [dict(name="magenta", top=LAYER_AT[0], left=LAYER_AT[1],
                      w=LAYER_SIZE[0], h=LAYER_SIZE[1], color=MAGENTA,
                      opacity=opacity)]
    else:
        # PSD 图层顺序：make_psd_fixture 里列表第一个 = 最底。
        specs = [
            dict(name="bg", top=0, left=0, w=CANVAS[0], h=CANVAS[1],
                 color=(0, 128, 128)),
            dict(name="magenta", top=LAYER_AT[0], left=LAYER_AT[1],
                 w=LAYER_SIZE[0], h=LAYER_SIZE[1], color=MAGENTA),
            dict(name="top", top=600, left=700, w=300, h=220,
                 color=(230, 220, 60)),
        ]
    mk.write_psd(path, CANVAS[0], CANVAS[1], specs)


# ------------------------------------------------------------------- win32 ----
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
    wr = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(wr))
    pt = wt.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(pt))
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


# --------------------------------------------------------------- measure ----
GMASK = 48  # 绿通道切阈值；半透明模式放宽到 110 以容纳棋盘格混合


def magenta_mass(img: Image.Image) -> tuple[float, int]:
    """返回 (洋红 alpha 加权质量, 洋红像素数)。"""
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    good = g <= GMASK
    w = np.minimum(r - g, b - g).astype(np.float32) / 255.0
    np.clip(w, 0.0, 1.0, out=w)
    w[~good] = 0.0
    mass = float(w.sum())
    count = int((w > 0.5).sum())
    return mass, count


def find_center(img: Image.Image):
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    w = np.minimum(r - g, b - g).astype(np.float32) / 255.0
    w[g > 110] = 0.0
    ys, xs = np.where(w > 0.15)
    if len(xs) == 0:
        return None
    return float(xs.mean()), float(ys.mean())


# ------------------------------------------------------------------- main ----
def main() -> int:
    global GMASK
    keep = "--keep" in sys.argv
    layers = 3 if "--layers" in sys.argv else 1
    circle = "--circle" in sys.argv
    opacity = 128 if "--alpha" in sys.argv else 255
    if "--alpha" in sys.argv:
        GMASK = 110
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    psd = os.path.join(TMP, "fade.psd")
    make_fixture(psd, layers, circle, opacity)

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
         "--script-after-paint=select-layer:%d" % (1 if layers == 1 else 2),
         "--script-delay=900"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1)

    samples: list[tuple[str, float, float, int]] = []  # phase, t, mass, count
    shots: list[tuple[str, float, Image.Image]] = []
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
        time.sleep(2.2 + (1.5 if layers > 1 else 0))  # 多层分层加载更久
        off = client_offset(hwnd)

        img = grab(hwnd)
        if img and keep:
            img.save(os.path.join(TMP, "debug_before_drag.png"))
        c = find_center(img) if img else None
        check("找到洋红块中心", c is not None,
              "max_w=%.2f" % (float(np.asarray(img.convert('RGB')).astype(np.int16)[:, :, 0].max()) if img else -1))
        if c is None:
            return 1
        cx, cy = c[0] - off[0], c[1] - off[1]

        m, n = magenta_mass(img)
        samples.append(("still", 0.0, m, n))
        if keep:
            shots.append(("still", 0.0, img))

        # 按下 + 分段移动
        post(hwnd, WM_MOUSEMOVE, cx, cy)
        time.sleep(0.05)
        post(hwnd, WM_LBUTTONDOWN, cx, cy, MK_LBUTTON)
        time.sleep(0.08)
        steps = 16
        for i in range(1, steps + 1):
            post(hwnd, WM_MOUSEMOVE, cx + MOVE[0] * i / steps,
                 cy + MOVE[1] * i / steps, MK_LBUTTON)
            time.sleep(0.03)
        # 最后一段停住
        post(hwnd, WM_MOUSEMOVE, cx + MOVE[0], cy + MOVE[1], MK_LBUTTON)
        time.sleep(0.10)
        img = grab(hwnd)
        if img:
            m, n = magenta_mass(img)
            samples.append(("drag", 0.0, m, n))
            if keep:
                shots.append(("drag", 0.0, img))

        # ---- 松手：高频连拍前后 1.2s ----
        t_release = time.perf_counter()
        post(hwnd, WM_LBUTTONUP, cx + MOVE[0], cy + MOVE[1], 0)
        while time.perf_counter() - t_release < 1.2:
            img = grab(hwnd)
            if img:
                t = time.perf_counter() - t_release
                m, n = magenta_mass(img)
                samples.append(("release", t, m, n))
                if keep:
                    shots.append(("release", t, img))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    drag_mass = next((m for p, _, m, _ in samples if p == "drag"), 0.0)
    rel = [(t, m, n) for p, t, m, n in samples if p == "release"]
    tail = sorted(m for _, m, _ in rel[-6:]) if rel else [0]
    settled = tail[len(tail) // 2]

    check("拖动中洋红块可见（走了 GPU 或 CPU 正常渲染）", drag_mass > 1000,
          "drag_mass=%.0f" % drag_mass)
    check("落定稳定后洋红块可见", settled > 1000, "settled=%.0f" % settled)

    # ★核心判据：拖动期（GPU 纹理）与落定期（CPU 预览）同一图层的 mass
    #   必须一致。bug 时 CPU 预览是预乘 BMP 被当 straight 显示 → 半透明层
    #   落定态 mass 只有拖动态的一半（用户看到「放手瞬间图层变透明」）。
    consist = settled / drag_mass if drag_mass else 999
    check("落定透明度与拖动期一致（mass 比 ∈ [0.85, 1.15]）",
          0.85 <= consist <= 1.15,
          "drag=%.0f settled=%.0f ratio=%.2f" % (drag_mass, settled, consist))

    # ★★核心回归：松手后的**中间帧**不允许出现 mass 尖峰。
    #   2026-11-25 用户报「放手瞬间混合区闪一下」：交叉淡出让 CPU 帧和 GPU
    #   三明治同显 90ms，半透明内容被 source-over 叠两次 —— mass 从 56921
    #   尖到 143738（2.5×）、像素数翻倍。旧判据只比 drag/settled 首尾（ratio
    #   =1.00）完全漏掉这个中间尖峰。这里改为检查每一帧，半透明场景尤其严。
    peak = max(rel, key=lambda x: x[1]) if rel else (0, 0, 0)
    peak_ratio = peak[1] / settled if settled else 999
    limit = 1.25 if "--alpha" in sys.argv else 1.4
    check("松手后每一帧 mass 都不超过基准（无交叉淡出双叠尖峰）",
          peak_ratio <= limit,
          "max_mass=%.0f@%.3fs settled=%.0f ratio=%.2f (limit=%.2f)" % (
              peak[1], peak[0], settled, peak_ratio, limit))
    # 半透明场景额外查像素数：双叠会让覆盖面积也翻倍（不只是变浓）
    if "--alpha" in sys.argv and rel:
        peak_n = max(n for _, _, n in rel)
        base_n = int(np.median([n for t, _, n in rel if t > 0.4])) if any(t > 0.4 for t, _, _ in rel) else peak_n
        check("松手后没有两份半透明内容同时可见（像素数不翻倍）",
              base_n == 0 or peak_n <= base_n * 1.3,
              "peak_n=%d base_n=%d" % (peak_n, base_n))

    print("\nsamples (phase, t, mass, count):")
    for p, t, m, n in samples:
        print("  %-8s %6.3f %10.0f %8d" % (p, t, m, n))

    if keep:
        for i, (phase, t, im) in enumerate(shots):
            im.save(os.path.join(TMP, "%02d_%s_%d.png" % (i, phase, int(t * 1000))))
        print("\nframes ->", TMP)
    else:
        shutil.rmtree(TMP, ignore_errors=True)

    print("\nRESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
