#!/usr/bin/env python
"""真鼠标端到端验证「拼图咬合」：元素拖到洞口时被洞壁 / 居中咬住。

为什么要有这个探针：`probe_gap_fit.py` 走的是 `--script` 直接调
`fit-probe` / `move-gesture`，验证的是**求解器**。用户报的「吸不进洞」
可能出在 QML 手势那一段（选中的图层、修饰键、显示比例换算）。所以这里
从真实窗口下手：真鼠标按下 → 多段移动 → 松手，最后读应用自己打印的
`lbox`（选中图层的外接框，图像坐标）判断落点。

坐标换算：`mark-press` 会报出画布算出的图像坐标 `ix,iy`，配合 `pad`
与 `scale`（keys-probe 里都有）可以反解「窗口客户区 → 图像坐标」的
仿射关系，不需要靠截图猜。

布局（图像坐标，1600×900）：
  * 照片型图层 600..1599，中间挖洞 800..1400 × 200..700；
  * 元素 = 300×300 的圆，画在 100..399 × 300..599（竖直方向正好落在洞
    的高度里，这样 X 轴投影才有意义）。

用法：
    python scripts/probe_fit_mouse.py
"""
import ctypes
import ctypes.wintypes as wt
import os
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
FIXTURE = os.path.join(ROOT, "probe_fit_mouse.png")
LOG = os.path.join(ROOT, "probe_fit_mouse.log")
SHOT = os.path.join(ROOT, "probe_fit_mouse_shot.png")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"

CANVAS_W, CANVAS_H = 1600, 900
ELEM = (100.0, 300.0, 300.0, 300.0)      # 元素内容框（图像坐标）
HOLE_L, HOLE_R = 800.0, 1400.0
HOLE_T, HOLE_B = 200.0, 700.0

u = ctypes.WinDLL("user32", use_last_error=True)
g = ctypes.WinDLL("gdi32")
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))

WM_MOUSEMOVE = 0x0200
WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
MK_LBUTTON = 0x0001


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


def post(hwnd, msg, x, y, wp=0):
    u.PostMessageW(hwnd, msg, wp, (int(y) << 16) | (int(x) & 0xFFFF))


def click(hwnd, x, y):
    post(hwnd, WM_MOUSEMOVE, x, y)
    time.sleep(0.04)
    post(hwnd, WM_LBUTTONDOWN, x, y, MK_LBUTTON)
    time.sleep(0.06)
    post(hwnd, WM_LBUTTONUP, x, y, 0)
    time.sleep(0.35)


def drag(hwnd, x0, y0, x1, y1, steps=16, settle=0.045):
    post(hwnd, WM_MOUSEMOVE, x0, y0)
    time.sleep(0.04)
    post(hwnd, WM_LBUTTONDOWN, x0, y0, MK_LBUTTON)
    time.sleep(0.06)
    for i in range(1, steps + 1):
        post(hwnd, WM_MOUSEMOVE, x0 + (x1 - x0) * i / steps,
             y0 + (y1 - y0) * i / steps, MK_LBUTTON)
        time.sleep(settle)
    # ★终点再补两次 move + 停顿：Qt 的事件压缩会把「最后一个 move」和松手
    #   合并，偶尔把落点吃掉一步（实测差 11 图像像素，刚好跳过 8px 吸附窗口）。
    post(hwnd, WM_MOUSEMOVE, x1, y1, MK_LBUTTON)
    time.sleep(0.12)
    post(hwnd, WM_MOUSEMOVE, x1, y1, MK_LBUTTON)
    time.sleep(0.06)
    post(hwnd, WM_LBUTTONUP, x1, y1, 0)
    time.sleep(0.5)


def fixture():
    from PIL import Image, ImageDraw
    img = Image.new("RGBA", (CANVAS_W, CANVAS_H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rectangle([600, 0, 1599, 899], fill=(255, 255, 255, 255))
    d.rectangle([HOLE_L, HOLE_T, HOLE_R - 1, HOLE_B - 1], fill=(0, 0, 0, 0))
    d.ellipse([ELEM[0], ELEM[1], ELEM[0] + ELEM[2] - 1, ELEM[1] + ELEM[3] - 1],
              fill=(200, 30, 40, 255))
    img.save(FIXTURE)


def log_text():
    try:
        with open(LOG, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def probes():
    out = []
    for ln in log_text().splitlines():
        if "keys-probe:" in ln:
            out.append(ln)
    return out


def press_lines():
    return [ln for ln in log_text().splitlines() if "mark-press:" in ln]


def stat(text, key):
    m = re.search(r"\b" + key + r"=([^ ]+)", text or "")
    return m.group(1) if m else "?"


def box(text, key):
    raw = stat(text, key)
    if raw in ("null", "?"):
        return None
    try:
        return tuple(float(v) for v in raw.split(","))
    except ValueError:
        return None


def wait_probe(after=0, timeout=3.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        lines = probes()
        if len(lines) > after:
            return lines[-1]
        time.sleep(0.15)
    lines = probes()
    return lines[-1] if lines else ""


def run_case(hwnd, box_now, target_left, win_fn, scale, expect_x, name):
    """把元素内容左边拖到 target_left，断言落定后的左边 == expect_x。"""
    cx, cy = box_now[0] + box_now[2] / 2, box_now[1] + box_now[3] / 2
    dx_img = target_left - box_now[0]
    # ★基线必须在**松手之后**取：keys-probe 每 1.1s 一行，拖动期间往往正好
    #   冒出一行（是中途位置），拿拖动前的行数当基线就会立刻读到它 ——
    #   落点看着时对时错（实测 841 / 863 / 934 全是拖动中途的框）。
    drag(hwnd, *win_fn(cx, cy), *win_fn(cx + dx_img, cy))
    line = wait_probe(len(probes()))
    b = box(line, "lbox")
    ok = b is not None and abs(b[0] - expect_x) <= 1.0
    print("  %-12s 拖 %+6.0f → lbox=%-18s 期望左边 %.0f  %s"
          % (name, dx_img,
             "null" if b is None else "%.0f,%.0f,%.0f,%.0f" % b,
             expect_x, "PASS" if ok else "FAIL"))
    return ok, b


def one_launch(cases):
    if os.path.exists(LOG):
        os.remove(LOG)
    args = [EXE, FIXTURE, "--probe-keys",
            "--script=extract:%d,%d,%d,%d" % (ELEM[0], ELEM[1], ELEM[2], ELEM[3])]
    env = dict(os.environ)
    env["PATH"] = QT_BIN + ";" + env.get("PATH", "")
    log = open(LOG, "w", encoding="utf-8")
    p = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=env, cwd=ROOT)
    try:
        time.sleep(3.5)
        hwnd = find_window(p.pid)
        if not hwnd:
            print("FAIL 没有窗口")
            return False
        cw, ch = client_size(hwnd)

        # 1) 校准：在客户区中间按一下，读出应用算出的图像坐标 + pad + scale。
        before = len(press_lines())
        probe0 = wait_probe(0)
        scale = float(stat(probe0, "scale"))
        pad = float(stat(probe0, "pad"))
        w0 = (cw // 2, ch // 2)
        click(hwnd, *w0)
        for _ in range(20):
            if len(press_lines()) > before:
                break
            time.sleep(0.15)
        pl = press_lines()
        if len(pl) <= before:
            print("FAIL 校准失败（没读到 mark-press）")
            return False
        m = re.search(r"ix=(-?\d+) iy=(-?\d+)", pl[-1])
        if not m:
            print("FAIL 校准行解析失败：%s" % pl[-1])
            return False
        ix, iy = int(m.group(1)), int(m.group(2))
        # window = pad + image*scale + C  →  C = w0 - pad - image*scale
        cxo = w0[0] - pad - ix * scale
        cyo = w0[1] - pad - iy * scale
        print("  校准：w0=%s → 图像 (%d,%d)，pad=%.0f scale=%.4f → C=(%.0f,%.0f)"
              % (w0, ix, iy, pad, scale, cxo, cyo))

        def win(px, py):
            return (int(round(pad + px * scale + cxo)),
                    int(round(pad + py * scale + cyo)))

        ok = True
        # 校准那一下会取消选中：点回元素中心，之后每轮都用当前框定位。
        click(hwnd, *win(ELEM[0] + ELEM[2] / 2, ELEM[1] + ELEM[3] / 2))
        cur = box(wait_probe(len(probes())), "lbox")
        if cur is None:
            print("FAIL 元素没被选中（lbox=null）")
            return False
        for name, target_left, expect_x in cases:
            good, nb = run_case(hwnd, cur, target_left, win, scale, expect_x, name)
            ok = ok and good
            if nb is None:
                return False
            cur = nb
        return ok
    finally:
        try:
            p.terminate()
        except Exception:
            pass
        log.close()


def main():
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fixture()
    # 元素内容 300 宽，初始左边 100；洞 800..1400（600 宽，居中左边 950）。
    # 1) 左边拖到 797（离洞壁 800 差 3）→ 咬到 800；
    # 2) 拖到 760（离墙 40）→ 不吸，落点 760；
    # 3) 拖到 945（居中 950，差 5）→ 居中咬到 950。
    cases = [
        ("贴洞壁", 797.0, HOLE_L),
        ("超出阈值不吸", 760.0, 760.0),
        ("洞内居中", 945.0, 950.0),
    ]
    ok = one_launch(cases)
    print()
    print("RESULT", "ALL PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
