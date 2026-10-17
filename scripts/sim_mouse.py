"""Simulate mouse drags on the imageedit window (SendInput) and capture
screenshots so selection interactions can be verified headlessly."""
import ctypes
import ctypes.wintypes as wt
import sys
import time

u = ctypes.windll.user32
u.SetProcessDPIAware()

# --- SendInput structures ---
LONG = ctypes.c_long


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", LONG), ("dy", LONG), ("mouseData", ctypes.c_ulong),
                ("dwFlags", ctypes.c_ulong), ("time", ctypes.c_ulong),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("union", _INPUTUNION)]


INPUT_MOUSE = 0
MOVE_ABS = 0x8000 | 0x0001   # MOUSEEVENTF_ABSOLUTE | MOVE
LEFTDOWN = 0x0002
LEFTUP = 0x0004


def send(dx, dy, flags):
    mi = MOUSEINPUT(dx, dy, 0, flags, 0, None)
    inp = INPUT(INPUT_MOUSE, _INPUTUNION(mi))
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def move_to(x, y):
    # absolute coords in 0..65535 virtual space
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
    time.sleep(0.12)


def window_rect(pid):
    res = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, _):
        pid_ = wt.DWORD()
        u.GetWindowThreadProcessId(h, ctypes.byref(pid_))
        if pid_.value == pid and u.IsWindowVisible(h):
            r = wt.RECT()
            u.GetWindowRect(h, ctypes.byref(r))
            res.append((r.left, r.top, r.right, r.bottom))
        return True
    u.EnumWindows(cb, 0)
    return res[0] if res else None


pid = int(sys.argv[1])
mode = sys.argv[2]  # "create" | "move"
r = window_rect(pid)
if not r:
    print("NO-WINDOW")
    sys.exit(1)
left, top, right, bottom = r
w, h = right - left, bottom - top
print("window", w, "x", h)

cx, cy = left + w // 2, top + h // 2

if mode == "create":
    # 1. create a selection inside the canvas
    drag(cx - 220, cy - 180, cx + 260, cy + 260)
else:
    # 2. press inside the selection and drag: the box should MOVE (fix #2)
    drag(cx - 60, cy - 60, cx + 120, cy + 40)

print("DONE")
