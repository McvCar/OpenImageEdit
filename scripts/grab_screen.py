"""用 GDI 直接抓「屏幕上的真实像素」——PrintWindow 对 D3D 合成的画布会返回旧帧。

usage: grab_screen.py <pid> <out.png>
按窗口客户区在屏幕上的位置截图（窗口必须可见、未被遮挡）。
"""
import ctypes
import sys
from ctypes import wintypes

from PIL import ImageGrab

user32 = ctypes.windll.user32
user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))  # PER_MONITOR_AWARE_V2


def main() -> int:
    pid = int(sys.argv[1])
    out = sys.argv[2]
    hwnd_found = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(hwnd, _lp):
        p = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(hwnd):
            hwnd_found.append(hwnd)
        return True

    user32.EnumWindows(cb, 0)
    if not hwnd_found:
        print("no window")
        return 1
    hwnd = hwnd_found[0]
    user32.SetForegroundWindow(hwnd)
    user32.BringWindowToTop(hwnd)
    import time

    time.sleep(0.5)
    r = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    print("rect", r.left, r.top, r.right, r.bottom)
    im = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom), all_screens=True)
    im.save(out)
    print("saved", im.size)
    return 0


if __name__ == "__main__":
    sys.exit(main())
