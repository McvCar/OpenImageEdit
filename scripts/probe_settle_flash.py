"""落定闪烁回归探针：移动图层**放手瞬间**，旧位置绝不能出现那一层。

背景（2026-11-22，用户两次报「移动图层放手瞬间旧位置闪一下」）：
Tier 1 GPU 拖动期间 `previewImage.opacity = 0`，屏幕靠三张 retained 纹理
（static / active / above）撑着。落定时 `gpuLive → false`，如果这里**立刻**
把 CPU 预览恢复成不透明，露出来的是**起手前那一帧**（图层还在原位）；
而 static/above 是「透明底合成」的全幅纹理 —— 图层原来占的那块本来就
是透明的，旧帧从透明处透出来，就是用户看到的那一层残影。

所以本探针不比对几何，只数「活动层颜色」的像素总量：

  * 正常（无论拖动中还是落定后）：屏幕上只有**一份**图层，count ≈ C；
  * 出 bug：GPU 纹理在新位置、CPU 旧帧在原位置同时可见 → count ≈ 2C。

用纯色块（洋红）+ 透明画布，判据是「任何一帧都不允许超过落定后那一帧的
1.3 倍」，跟窗口位置 / 缩放 / DPI 全解耦。

usage: python scripts/probe_settle_flash.py [--keep] [--frames N]
"""
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageStat

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = sys.executable
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_settle_flash")
PSD = os.path.join(TMP, "flash.psd")

CANVAS = (3000, 2000)
LAYER_AT = (400, 400)          # top, left
LAYER_SIZE = (1200, 800)       # w, h
MOVE = (1200, 700)             # dx, dy
MAGENTA = (255, 0, 255)

user32 = ctypes.windll.user32
gdi = ctypes.windll.gdi32
PW_RENDERFULLCONTENT = 0x2

ok = True


def check(name: str, cond: bool, extra: str = "") -> None:
    global ok
    ok = ok and cond
    print(("PASS " if cond else "FAIL ") + name + (("  " + extra) if extra else ""))


# ---------------------------------------------------------------- fixture ----
def make_fixture() -> None:
    """600x400 透明画布 + 一个洋红实心块（无蒙版 / Normal / 满不透明 → 走 GPU）。"""
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    import make_psd_fixture as mk

    def channel_data(layer, ci):
        w, h = layer["w"], layer["h"]
        if ci == 3:                                # transparency mask
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


# --------------------------------------------------------------- capture ----
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


def magenta_count(img: Image.Image) -> int:
    """洋红像素数（±60 容差，够区分选框 / 手柄的蓝白）。用 LUT + multiply 走 C 速度。"""
    r, g, b = img.split()
    rm = r.point(lambda v: 255 if v > 195 else 0)
    gm = g.point(lambda v: 255 if v < 60 else 0)
    bm = b.point(lambda v: 255 if v > 195 else 0)
    m = ImageChops.multiply(ImageChops.multiply(rm, gm), bm)
    return int(ImageStat.Stat(m).sum[0] / 255)


# ------------------------------------------------------------------ main ----
def main() -> int:
    import shutil

    frames_wanted = 120
    if "--frames" in sys.argv:
        frames_wanted = int(sys.argv[sys.argv.index("--frames") + 1])

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
    script = "select-layer:1;gpu-gesture:%d,%d" % MOVE
    log = open(os.path.join(TMP, "run.log"), "w", encoding="utf-8")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), PSD,
         "--script-after-paint=" + script, "--script-delay=900"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1,
    )

    t0 = time.perf_counter()
    try:
        # 等主窗口出现（脚本基本在窗口显示前就跑完了，所以这里只能等窗口）。
        hwnd = None
        while time.perf_counter() - t0 < 15:
            hwnd = find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        check("主窗口已出现", hwnd is not None)
        if hwnd is None:
            return 1

        # 窗口一出现就开始连拍：脚本用 `--script-after-paint` 推迟到第一帧上屏
        # 之后才跑，所以落定 / 淡出就发生在连拍开始后的这几百毫秒里。
        counts: list[tuple[float, int]] = []
        keep_shots: list[tuple[float, Image.Image]] = []
        deadline = time.perf_counter() + 6.0
        while time.perf_counter() < deadline:
            img = grab(hwnd)
            if img is None:
                continue
            t = time.perf_counter() - t0
            c = magenta_count(img)
            counts.append((t, c))
            if "--keep" in sys.argv:
                keep_shots.append((t, img))
            if len(counts) >= frames_wanted:
                break
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()

    log_text = open(os.path.join(TMP, "run.log"), encoding="utf-8", errors="replace").read()
    check("手势完成", '"event":"gpu-gesture","began":true,"landed":true' in log_text)
    check("走了 GPU 三明治快路径",
          '"event":"gpu-begin"' in log_text or '"event":"gpu-begin-set"' in log_text)

    check("拍到足够多的帧", len(counts) >= 15, "%d 帧 / 跨度 %.2fs" % (
        len(counts), counts[-1][0] if counts else 0))

    # 落定后稳定态的那份图层像素数（取最后几帧的中位数，避开截到一半的画面）
    tail = sorted(c for _, c in counts[-5:])
    settled_count = tail[len(tail) // 2] if tail else 0
    check("最后落在新位置（洋红可见）", settled_count > 40, "count=%d" % settled_count)

    worst = max(counts, key=lambda tc: tc[1]) if counts else (0, 0)
    ratio = (worst[1] / settled_count) if settled_count else 999
    check("旧位置从未闪出第二份图层（任何帧 count 不超过落定值 1.3×）",
          ratio <= 1.3,
          "peak=%d@%.2fs settled=%d ratio=%.2f" % (worst[1], worst[0], settled_count, ratio))

    # 起手换 GPU 纹理的那一两帧：三张 Image 是异步解码的，空窗期如果 CPU
    # 预览已经让位，整块画布会空白 —— 同样是「闪一下」。
    first_on = next((i for i, (_, c) in enumerate(counts) if c > 0), None)
    if first_on is not None and settled_count > 0:
        after = [c for _, c in counts[first_on:]]
        low = min(after)
        check("起手/落定全程没有整块空白（图层不会消失一两帧）",
              low >= settled_count * 0.5,
              "min=%d settled=%d（首个非空帧之后 %d 帧）" % (low, settled_count, len(after)))

    print("counts:", " ".join("%.2f:%d" % (t, c) for t, c in counts))
    if "--keep" in sys.argv:
        for i, (t, im) in enumerate(keep_shots):
            im.save(os.path.join(TMP, "f%02d_%d.png" % (i, int(t * 1000))))
        print("frames ->", TMP)
    else:
        shutil.rmtree(TMP, ignore_errors=True)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
