"""Click at window-relative coordinates of the imageedit main window.

Usage: python ie_click.py <pid> <x> <y> [<x> <y> ...] [--shot out.png]
Coordinates are relative to the window top-left (client area of the
PrintWindow screenshot), so they match screenshots taken by shot_window.py.
"""
import ctypes
import ctypes.wintypes as wt
import sys
import time

u = ctypes.windll.user32
u.SetProcessDPIAware()

LONG = ctypes.c_long


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", LONG), ("dy", LONG), ("mouseData", ctypes.c_ulong),
                ("dwFlags", ctypes.c_ulong), ("time", ctypes.c_ulong),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("union", _INPUTUNION)]


MOVE_ABS = 0x8000 | 0x0001
LEFTDOWN = 0x0002
LEFTUP = 0x0004


def send(dx, dy, flags):
    mi = MOUSEINPUT(dx, dy, 0, flags, 0, None)
    inp = INPUT(0, _INPUTUNION(mi))
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def move_to(sx, sy):
    send(int(sx * 65535 / u.GetSystemMetrics(0)),
         int(sy * 65535 / u.GetSystemMetrics(1)), MOVE_ABS)


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


def main():
    args = sys.argv[1:]
    pid = int(args[0])
    rest = args[1:]
    shot = None
    if "--shot" in rest:
        i = rest.index("--shot")
        shot = rest[i + 1]
        rest = rest[:i] + rest[i + 2:]

    r = window_rect(pid)
    if not r:
        print("NO-WINDOW")
        sys.exit(1)
    left, top, _, _ = r

    coords = [(int(rest[i]), int(rest[i + 1])) for i in range(0, len(rest), 2)]
    for (x, y) in coords:
        move_to(left + x, top + y)
        time.sleep(0.12)
        send(0, 0, LEFTDOWN)
        time.sleep(0.06)
        send(0, 0, LEFTUP)
        time.sleep(0.2)
        print("clicked", x, y)

    if shot:
        time.sleep(0.4)
        import PIL.ImageGrab
        PIL.ImageGrab.grab(all_screens=True).save(shot)
        print("shot", shot)


if __name__ == "__main__":
    main()
