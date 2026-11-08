#!/usr/bin/env python
"""复现「拖动 + 缩放之后删不掉」：真鼠标事件 + 真实 Delete 按键。

三种用户场景（--scenario rect|text|layer）：
  1. rect  ：--probe-marks 把工具切成矩形，在空白处拖出一个矩形标记；
  2. text  ：--probe-font 在图片中心放一个文本标记（等价于点工具条的 T）；
  3. layer ：--script=extract:… 把一块区域抠成普通图层。
之后统一走「按住元素拖一段 → 按住右下角手柄缩一段 → 投递真实 Delete」。

★坐标要自校准：PostMessage 投给 Qt 的客户区坐标和 PrintWindow 截图里的
像素之间有一个固定偏移（实测 x 差 60~125px），拿截图矩形直接换算会漏点
小目标（55×36 的文字标记就点不中）。所以先按一下，让应用（--probe-keys
下的 mark-press 行）报出它算出的图像坐标，反推出偏移 C，再用 C 定位。

用法：
    python scripts/probe_mark_delete.py --scenario rect
    python scripts/probe_mark_delete.py --scenario text
    python scripts/probe_mark_delete.py --scenario layer
    python scripts/probe_mark_delete.py --scenario rect --focus
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
LOG = os.path.join(ROOT, "probe_mark_delete.log")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
FIXTURE = os.path.join(ROOT, "probe_mark_fixture.png")

u = ctypes.WinDLL("user32", use_last_error=True)
g = ctypes.WinDLL("gdi32")
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))

WM_MOUSEMOVE = 0x0200
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101
MK_LBUTTON = 0x0001
PW_RENDERFULLCONTENT = 0x2


def find_window(pid):
    out = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hwnd, _l):
        p = wt.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and u.IsWindowVisible(hwnd) and u.GetWindowTextLengthW(hwnd) > 0:
            out.append(hwnd)
        return True

    u.EnumWindows(cb, 0)
    return out[0] if out else None


def client_size(hwnd):
    r = wt.RECT()
    u.GetClientRect(hwnd, ctypes.byref(r))
    return r.right - r.left, r.bottom - r.top


def screenshot(hwnd, path):
    from PIL import Image

    w, h = client_size(hwnd)
    hdc = u.GetWindowDC(hwnd)
    mem = g.CreateCompatibleDC(hdc)
    bmp = g.CreateCompatibleBitmap(hdc, w, h)
    g.SelectObject(mem, bmp)
    u.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)

    class BMIH(ctypes.Structure):
        _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                    ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                    ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                    ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                    ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                    ("biClrImportant", ctypes.c_uint32)]

    bmi = BMIH(ctypes.sizeof(BMIH), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(w * h * 4)
    g.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bmi), 0)
    img = Image.frombuffer("RGBA", (w, h), buf.raw, "raw", "BGRA", 0, 1)
    img.save(path)
    g.DeleteObject(bmp)
    g.DeleteDC(mem)
    u.ReleaseDC(hwnd, hdc)
    return img


def detect_image_rect(img):
    """图片是浅色（238）的大块，界面是深色：只认「整行 / 整列有很多亮像素」，
    浮层、白字、标记那些零碎亮点撑不大包围盒。"""
    rgb = img.convert("RGB")
    w, h = rgb.size
    px = rgb.load()
    bright = [[False] * w for _ in range(h)]
    for y in range(h):
        row = bright[y]
        for x in range(w):
            r, g_, b = px[x, y]
            row[x] = r > 200 and g_ > 200 and b > 200
    rows = [y for y in range(h) if sum(bright[y]) >= 200]
    if not rows:
        return None
    ymin, ymax = rows[0], rows[-1]
    cols = [x for x in range(w)
            if sum(1 for y in range(ymin, ymax + 1) if bright[y][x]) >= 120]
    if not cols:
        return None
    return (cols[0], ymin, cols[-1], ymax)


def post(hwnd, msg, x, y, wp=0):
    u.PostMessageW(hwnd, msg, wp, (int(y) << 16) | (int(x) & 0xFFFF))


def click(hwnd, x, y):
    post(hwnd, WM_MOUSEMOVE, x, y)
    time.sleep(0.04)
    post(hwnd, WM_LBUTTONDOWN, x, y, MK_LBUTTON)
    time.sleep(0.06)
    post(hwnd, WM_LBUTTONUP, x, y, 0)
    time.sleep(0.35)


def drag(hwnd, x0, y0, x1, y1, steps=14, settle=0.05):
    post(hwnd, WM_MOUSEMOVE, x0, y0)
    time.sleep(0.04)
    post(hwnd, WM_LBUTTONDOWN, x0, y0, MK_LBUTTON)
    time.sleep(0.06)
    for i in range(1, steps + 1):
        post(hwnd, WM_MOUSEMOVE, x0 + (x1 - x0) * i / steps,
             y0 + (y1 - y0) * i / steps, MK_LBUTTON)
        time.sleep(settle)
    post(hwnd, WM_LBUTTONUP, x1, y1, 0)
    time.sleep(0.4)


def press_key(hwnd, vk=0x2E, hold=0.05):
    sc = u.MapVirtualKeyW(vk, 0)

    def lp(up):
        v = 1 | (sc << 16)
        if up:
            v |= (1 << 30) | (1 << 31)
        return ctypes.c_long(v).value

    u.PostMessageW(hwnd, WM_KEYDOWN, vk, lp(False))
    time.sleep(hold)
    u.PostMessageW(hwnd, WM_KEYUP, vk, lp(True))


def log_text():
    try:
        with open(LOG, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def probe_lines():
    out = []
    for ln in log_text().splitlines():
        if '"event":"mark"' in ln and "keys-probe:" in ln:
            try:
                out.append(json.loads(ln.strip())["name"])
            except Exception:
                pass
    return out


def press_lines():
    out = []
    for ln in log_text().splitlines():
        if "mark-press:" in ln:
            out.append(ln)
    return out


def stat(text, key):
    m = re.search(r"\b" + key + r"=([^ ]+)", text or "")
    return m.group(1) if m else "?"


def dump(tag):
    time.sleep(1.4)
    lines = probe_lines()
    s = lines[-1] if lines else ""
    print("%-12s m=%s mk=%s del=%s v=%s sizeFocus=%s mbox=%s lbox=%s layers=%s"
          % (tag, stat(s, "selMark"), stat(s, "marks"), stat(s, "delEnabled"),
             stat(s, "vEnabled"), stat(s, "sizeFocus"),
             stat(s, "mbox"), stat(s, "lbox"), stat(s, "layers")))
    return s


def fixture(path):
    from PIL import Image, ImageDraw
    im = Image.new("RGB", (600, 400), (238, 238, 238))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 599, 399], outline=(220, 40, 40), width=3)
    for x in range(40, 600, 40):
        d.line([(x, 3), (x, 396)], fill=(226, 226, 226))
    im.save(path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenario", default="rect", choices=["rect", "text", "layer"])
    ap.add_argument("--exe", default=EXE)
    ap.add_argument("--focus", action="store_true",
                    help="先用 --probe-keys-focus 造出字号框焦点，再点画布验证焦点被收走")
    a = ap.parse_args()

    fixture(FIXTURE)
    if os.path.exists(LOG):
        os.remove(LOG)
    args = [a.exe, FIXTURE, "--probe-keys", "--no-restore-tool"]
    if a.focus:
        args.append("--probe-keys-focus")
    if a.scenario == "rect":
        args.append("--probe-marks")
    elif a.scenario == "text":
        args.append("--probe-font")
    else:
        args.append("--script=extract:150,120,200,160")

    env = dict(os.environ)
    env["PATH"] = QT_BIN + ";" + env.get("PATH", "")
    log = open(LOG, "w", encoding="utf-8")
    p = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=env, cwd=ROOT)
    try:
        time.sleep(4.0)
        hwnd = find_window(p.pid)
        if not hwnd:
            print("FAIL 没有窗口")
            return 1

        img = screenshot(hwnd, os.path.join(ROOT, "probe_mark_delete.png"))
        rect = detect_image_rect(img)
        if not rect:
            print("FAIL 找不到图像矩形")
            return 1
        rx, ry, rx1, ry1 = rect
        rough = lambda ix, iy: (int(rx + ix * (rx1 - rx) / 600.0),
                                int(ry + iy * (ry1 - ry) / 400.0))
        print("图像矩形(截图)=%s" % (rect,))

        def calibrate():
            """按一下让应用报出它算出的图像坐标，反推窗口偏移。"""
            before = len(press_lines())
            w0 = rough(12, 12)
            click(hwnd, w0[0], w0[1])
            for _ in range(8):
                if len(press_lines()) > before:
                    break
                time.sleep(0.2)
            lines = press_lines()
            if len(lines) <= before:
                return None
            m = re.search(r"ix=(-?\d+) iy=(-?\d+)", lines[-1])
            if not m:
                return None
            ix, iy = int(m.group(1)), int(m.group(2))
            return (w0[0] - ix, w0[1] - iy)

        if a.focus:
            for _ in range(40):
                if stat(probe_lines()[-1] if probe_lines() else "", "sizeFocus") == "true":
                    break
                time.sleep(0.25)
            cur = probe_lines()[-1] if probe_lines() else ""
            print("焦点已造出：sizeFocus=%s vEnabled=%s delEnabled=%s"
                  % (stat(cur, "sizeFocus"), stat(cur, "vEnabled"),
                     stat(cur, "delEnabled")))
            click(hwnd, *rough(40, 40))
            after_click = dump("点画布后")
            ok = stat(after_click, "sizeFocus") == "false" and \
                stat(after_click, "vEnabled") == "true"
            print("FOCUS-FIX", "PASS" if ok else "FAIL")

        if a.scenario == "text":
            # 字体面板打开时有一层 z=215 的点击捕获层，第一下只会关面板。
            click(hwnd, *rough(40, 40))
            time.sleep(0.4)

        # 启动时元素可能已经选中（text/layer 场景），先把框记下来：
        # 校准那一按会点空白把它取消选中，后面用它点回去。
        key = "lbox" if a.scenario == "layer" else "mbox"
        pre = dump("启动后")
        prebox = stat(pre, key)
        prebox = None if prebox in ("null", "?") else tuple(float(v) for v in prebox.split(","))

        C = calibrate()
        if C is None:
            print("FAIL 校准失败（没读到 mark-press 行）")
            return 1
        print("窗口偏移 C=%s" % (C,))

        def win(ix, iy):
            return C[0] + int(round(ix)), C[1] + int(round(iy))

        if prebox is not None:
            click(hwnd, *win(prebox[0] + prebox[2] / 2, prebox[1] + prebox[3] / 2))
        elif a.scenario == "rect":
            drag(hwnd, *win(60, 60), *win(200, 160))

        name = {"rect": "画矩形", "text": "建文本", "layer": "抠图层"}[a.scenario]
        first = dump("%s后" % name)
        raw = stat(first, key)
        if raw in ("null", "?"):
            print("FAIL 没读到 %s（%s）" % (key, raw))
            return 1
        box = tuple(float(v) for v in raw.split(","))
        print("用日志里的框 %s=%s 定位" % (key, raw))
        cx, cy = box[0] + box[2] / 2, box[1] + box[3] / 2

        if a.scenario == "layer":
            click(hwnd, *win(40, 40))
            gone = dump("点空白后")
            print("CLICK-EMPTY", "PASS" if stat(gone, "selLayer") == "-1" else "FAIL")
            click(hwnd, *win(cx, cy))
            dump("重选图层后")

        drag(hwnd, *win(cx, cy), *win(cx + 40, cy + 30))
        dump("拖动后")

        se = (box[0] + box[2] + 40, box[1] + box[3] + 30)
        drag(hwnd, *win(se[0], se[1]), *win(se[0] + 50, se[1] + 40))
        dump("缩放后")

        u.SetForegroundWindow(hwnd)
        time.sleep(0.3)
        press_key(hwnd)
        after = dump("Delete后")
        screenshot(hwnd, os.path.join(ROOT, "probe_mark_delete.png"))
        cnt = stat(after, "layers") if a.scenario == "layer" else stat(after, "marks")
        print("RESULT", "DELETED" if cnt == "0" else "STILL-THERE",
              "| marks=%s layers=%s selMark=%s delEnabled=%s"
              % (stat(after, "marks"), stat(after, "layers"),
                 stat(after, "selMark"), stat(after, "delEnabled")))
        return 0
    finally:
        try:
            p.terminate()
        except Exception:
            pass
        log.close()


if __name__ == "__main__":
    sys.exit(main())
