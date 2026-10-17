"""Robust cursor verification:
1. fresh app state, create a selection via SendInput drag
2. screenshot, locate the dashed box edges by scanning for black pixel runs
3. sample the live cursor at measured edge/corner/interior/outside points
"""
import ctypes
import ctypes.wintypes as wt
import sys
import time

u = ctypes.windll.user32
u.SetProcessDPIAware()


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", ctypes.c_long), ("dy", ctypes.c_long),
                ("mouseData", ctypes.c_ulong), ("dwFlags", ctypes.c_ulong),
                ("time", ctypes.c_ulong),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class _U(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("u", _U)]


MOVE_ABS = 0x8000 | 0x0001
LEFTDOWN = 0x0002
LEFTUP = 0x0004


def send(dx, dy, flags):
    u.SendInput(1, ctypes.byref(INPUT(0, _U(MOUSEINPUT(dx, dy, 0, flags, 0, None)))),
                ctypes.sizeof(INPUT))


def move_to(x, y):
    send(int(x * 65535 / u.GetSystemMetrics(0)),
         int(y * 65535 / u.GetSystemMetrics(1)), MOVE_ABS)


def drag(x1, y1, x2, y2, steps=20):
    move_to(x1, y1)
    time.sleep(0.1)
    send(0, 0, LEFTDOWN)
    time.sleep(0.06)
    for i in range(1, steps + 1):
        move_to(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps)
        time.sleep(0.01)
    send(0, 0, LEFTUP)
    time.sleep(0.2)


IDC_RES = {"ARROW": 32512, "CROSS": 32515, "SIZEALL": 32646,
           "SIZENESW": 32643, "SIZENS": 32645, "SIZENWSE": 32642,
           "SIZEWE": 32644}
_BY_HANDLE = {}
for _n, _r in IDC_RES.items():
    _BY_HANDLE[u.LoadCursorW(None, ctypes.c_void_p(_r)) or 0] = _n


class CURSORINFO(ctypes.Structure):
    _fields_ = [("cbSize", ctypes.c_ulong), ("flags", ctypes.c_ulong),
                ("hCursor", ctypes.c_void_p), ("ptScreenPos", wt.POINT)]


def cursor_at():
    """Return (cursor_name, screen_pos) of the current global cursor."""
    ci = CURSORINFO()
    ci.cbSize = ctypes.sizeof(CURSORINFO)
    u.GetCursorInfo(ctypes.byref(ci))
    name = _BY_HANDLE.get(ci.hCursor or 0,
                          "UNKNOWN(%s)" % hex(ci.hCursor or 0))
    return name, (ci.ptScreenPos.x, ci.ptScreenPos.y)


def window_rect(pid):
    res = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, _):
        p = wt.DWORD()
        u.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and u.IsWindowVisible(h):
            r = wt.RECT()
            u.GetWindowRect(h, ctypes.byref(r))
            res.append((r.left, r.top, r.right, r.bottom))
        return True
    u.EnumWindows(cb, 0)
    return res[0] if res else None


# ---------- PIL: locate the dashed box ----------
from PIL import Image


def find_box(png, x0, y0, x1, y1, exp):
    """Scan a +-25px band around each expected box edge for the column/row
    with the most near-black pixels; return measured (l, t, r, b)."""
    im = Image.open(png).convert("L")
    px = im.load()
    band = 25

    def col_dark(cx, ry0, ry1):
        return sum(1 for y in range(ry0, ry1) if px[cx, y] < 60)

    def row_dark(cy, rx0, rx1):
        return sum(1 for x in range(rx0, rx1) if px[x, cy] < 60)

    l, t, r, b = exp
    # vertical edges: search within [y0+40, y1-40]
    best = max(range(l - band, l + band),
               key=lambda cx: col_dark(cx, y0 + 40, y1 - 40))
    ml = best
    best = max(range(r - band, r + band),
               key=lambda cx: col_dark(cx, y0 + 40, y1 - 40))
    mr = best
    best = max(range(t - band, t + band),
               key=lambda cy: row_dark(cy, x0 + 40, x1 - 40))
    mt = best
    best = max(range(b - band, b + band),
               key=lambda cy: row_dark(cy, x0 + 40, x1 - 40))
    mb = best
    return ml, mt, mr, mb


def shot(pid, path):
    r = window_rect(pid)
    l, t, rr, bb = r
    import ctypes as c
    user32 = c.windll.user32
    hdc = user32.GetWindowDC(c.c_int(l) if False else None)  # unused
    return r


pid = int(sys.argv[1])
png = sys.argv[2]

# --- pin window and create selection ---
l, t, rr, bb = window_rect(pid)
u.SetWindowPos(ctypes.windll.user32.GetForegroundWindow(), 0, 0, 0, 0, 0, 0)


@ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
def pin(h, _):
    p = wt.DWORD()
    u.GetWindowThreadProcessId(h, ctypes.byref(p))
    if p.value == pid and u.IsWindowVisible(h):
        u.SetWindowPos(h, 0, 100, 100, 1100, 760, 0x0040)
        u.SetForegroundWindow(h)
    return True
u.EnumWindows(pin, 0)
time.sleep(0.5)

l, t, rr, bb = window_rect(pid)
print("window @", l, t, rr - l, bb - t)

# image display area ~ x l+88..l+1013, y t+38..t+732 (fit 58%)
IX0, IY0, IX1, IY1 = l + 88, t + 38, l + 1013, t + 732
drag(IX0 + 300, IY0 + 200, IX0 + 640, IY0 + 480)
exp = (IX0 + 300, IY0 + 200, IX0 + 640, IY0 + 480)

# --- screenshot & measure (PNG is window-local; subtract window origin) ---
import subprocess
subprocess.run(["C:/Users/Admin/.workbuddy/binaries/python/envs/default/"
                "Scripts/python.exe", "scripts/shot_window.py", str(pid), png],
               check=True)
ml, mt, mr, mb = find_box(png, IX0 - l, IY0 - t, IX1 - l, IY1 - t,
                          (exp[0] - l, exp[1] - t, exp[2] - l, exp[3] - t))
print("expected box (screen):", exp)
print("measured box (screen):", ml + l, mt + t, mr + l, mb + t)

if min(ml, mt, mr, mb) <= 0 or mr - ml < 50 or mb - mt < 50:
    print("BOX-DETECT-FAILED")
    sys.exit(1)

# convert local -> screen for sampling
ml += l; mt += t; mr += l; mb += t

mid = lambda a, b: (a + b) // 2
samples = [
    ("left-edge ", ml, mid(mt, mb), "SIZEWE"),
    ("right-edge", mr, mid(mt, mb), "SIZEWE"),
    ("top-edge  ", mid(ml, mr), mt, "SIZENS"),
    ("bottom    ", mid(ml, mr), mb, "SIZENS"),
    ("corner-nw ", ml, mt, "SIZENWSE"),
    ("corner-se ", mr, mb, "SIZENWSE"),
    ("interior  ", mid(ml, mr) + 40, mid(mt, mb) + 30, "SIZEALL"),
    ("outside   ", ml - 50, mid(mt, mb), "CROSS"),
]
bad = 0
for label, x, y, expect in samples:
    move_to(x, y)
    time.sleep(0.35)
    got, pos = cursor_at()
    on_target = abs(pos[0] - x) <= 2 and abs(pos[1] - y) <= 2
    ok = got == expect and on_target
    bad += 0 if ok else 1
    tag = "OK" if ok else ("MOUSE-OFFTARGET" if not on_target else "BAD")
    print("%s expect=%-8s got=%-12s pos=%s %s"
          % (label, expect, got, pos, tag))
print("RESULT:", "ALL-OK" if bad == 0 else "%d BAD" % bad)
