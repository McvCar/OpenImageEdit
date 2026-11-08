"""空窗口启动（无文件参数）是否居中在光标。"""
import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time

u32 = ctypes.WinDLL("user32", use_last_error=True)
u32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
E = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def wins_of(pid):
    out = []

    def cb(h, _):
        p = wt.DWORD()
        u32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and u32.IsWindowVisible(h):
            r = wt.RECT()
            u32.GetWindowRect(h, ctypes.byref(r))
            out.append((r.left, r.top, r.right - r.left, r.bottom - r.top))
        return True

    u32.EnumWindows(E(cb), 0)
    return out


env = dict(os.environ)
env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
exe = r"D:\_APP_CODE\ImageEdit\target\debug\imageedit.exe"
spot = (int(sys.argv[1]), int(sys.argv[2]))
u32.SetCursorPos(*spot)
time.sleep(0.2)
pt = wt.POINT()
u32.GetCursorPos(ctypes.byref(pt))
args = [exe] + sys.argv[3:]
p = subprocess.Popen(args, env=env)
for _ in range(50):
    time.sleep(0.1)
    if wins_of(p.pid):
        break
time.sleep(0.6)
print(f"光标=({pt.x},{pt.y})")
for x, y, w, h in wins_of(p.pid):
    print(f"  窗口 ({x},{y}) {w}x{h} 中心=({x + w // 2},{y + h // 2})"
          f" 偏差=({x + w // 2 - pt.x},{y + h // 2 - pt.y})")
p.terminate()
