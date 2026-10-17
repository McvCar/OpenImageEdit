"""Verify the app's live cursor at points around the selection box.
Creates a selection, then samples GetCursorInfo at edge/corner/interior/
outside points and maps the cursor handle to its IDC_ name."""
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
    u.SendInput(1, ctypes.byref(INPUT(0, _U(mi))), ctypes.sizeof(INPUT))


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
    time.sleep(0.15)


IDC_RES = {"ARROW": 32512, "CROSS": 32515, "SIZEALL": 32646,
           "SIZENESW": 32643, "SIZENS": 32645, "SIZENWSE": 32642,
           "SIZEWE": 32644}
# Load the stock cursors once; handles are deterministic for this session.
_BY_HANDLE = {}
for _name, _rid in IDC_RES.items():
    _BY_HANDLE[u.LoadCursorW(None, _rid) or 0] = _name


class CURSORINFO(ctypes.Structure):
    _fields_ = [("cbSize", ctypes.c_ulong), ("flags", ctypes.c_ulong),
                ("hCursor", ctypes.c_void_p),
                ("ptScreenPos", wt.POINT)]


def cursor_name():
    ci = CURSORINFO()
    ci.cbSize = ctypes.sizeof(CURSORINFO)
    u.GetCursorInfo(ctypes.byref(ci))
    return _BY_HANDLE.get(ci.hCursor or 0,
                          "UNKNOWN(%s)" % hex(ci.hCursor or 0))


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
r = window_rect(pid)
if not r:
    print("NO-WINDOW")
    sys.exit(1)
left, top, right, bottom = r
print("window @", left, top)

# Bring to front so the app owns the cursor.
u.SetForegroundWindow(__import__("ctypes").windll.user32.FindWindowW(None, None)) if False else None


@ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
def fg(h, _):
    p = wt.DWORD()
    u.GetWindowThreadProcessId(h, ctypes.byref(p))
    if p.value == pid and u.IsWindowVisible(h):
        u.SetForegroundWindow(h)
    return True
u.EnumWindows(fg, 0)
time.sleep(0.4)

# Image display area at fit zoom (window 1100x760): x 188..1113, y 138..832.
IMG_L, IMG_T, IMG_R, IMG_B = left + 188, top + 138, left + 1113, top + 832

# Create a mid-canvas selection.
drag(IMG_L + 300, IMG_T + 200, IMG_L + 640, IMG_T + 480)
bx1, by1, bx2, by2 = IMG_L + 300, IMG_T + 200, IMG_L + 640, IMG_T + 480
mid = lambda a, b: (a + b) // 2

samples = [
    ("left-edge   ", mid(bx1, bx2) - 170, mid(by1, by2), "SIZEWE"),
    ("top-edge    ", mid(bx1, bx2), by1 - 4, "SIZENS"),
    ("corner-nw   ", bx1, by1, "SIZENWSE"),
    ("corner-se   ", bx2, by2, "SIZENWSE"),
    ("interior    ", mid(bx1, bx2) + 60, mid(by1, by2) + 40, "SIZEALL"),
    ("outside     ", bx1 - 60, mid(by1, by2), "CROSS"),
]
for label, x, y, expect in samples:
    move_to(x, y)
    time.sleep(0.25)
    got = cursor_name()
    mark = "OK " if expect in got or got == expect else "BAD"
    print("%s expect=%-8s got=%s %s" % (label, expect, got, mark))
print("CHECK-DONE")
