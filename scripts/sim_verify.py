"""One-shot verification: create an overshooting selection (clamped to the
canvas), then drag its left edge strip to widen it. Box coords are derived
from the drag commands themselves, not from screenshots."""
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
    mi = MOUSEINPUT(dx, dy, 0, flags, 0, None)
    inp = INPUT(0, _U(mi))
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def move_to(x, y):
    send(int(x * 65535 / u.GetSystemMetrics(0)),
         int(y * 65535 / u.GetSystemMetrics(1)), MOVE_ABS)


def drag(x1, y1, x2, y2, steps=25):
    move_to(x1, y1)
    time.sleep(0.12)
    send(0, 0, LEFTDOWN)
    time.sleep(0.08)
    for i in range(1, steps + 1):
        move_to(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps)
        time.sleep(0.012)
    time.sleep(0.08)
    send(0, 0, LEFTUP)
    time.sleep(0.15)


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


pid = int(sys.argv[1])
shot = sys.argv[2]
r = window_rect(pid)
if not r:
    print("NO-WINDOW")
    sys.exit(1)
left, top, right, bottom = r

# Image display area at fit zoom for 1600x1200 in a 1100x760 window:
# canvas y 34..736, image 925x694 at x 88..1013, y 38..732.
IMG_L, IMG_T, IMG_R, IMG_B = left + 88, top + 38, left + 1013, top + 732

# 1. Overshooting rubber band: ends far outside the canvas → must clamp.
drag(IMG_L + 110, IMG_T + 110, right + 200, bottom + 200)
time.sleep(0.2)

# Box after clamping (display px, window coords).
bx1, by1, bx2, by2 = IMG_L + 110, IMG_T + 110, IMG_R, IMG_B

# 2. Grab the LEFT edge strip at the box's vertical midpoint, drag left 50px.
mid_y = (by1 + by2) // 2
drag(bx1, mid_y, bx1 - 50, mid_y)
print("box-after-create: %d,%d -> %d,%d" % (bx1, by1, bx2, by2))
print("DONE", shot)
