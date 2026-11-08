"""窗口定位端到端验证：
1) 光标在 A 点启动主窗口 -> 中心必须 = A（之后把鼠标挪开也不许跟着跑）
2) 光标在 B/C 点再双击别的图（单实例转发）-> 每个新窗口中心都必须 = B/C
"""
import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import time

u32 = ctypes.WinDLL("user32", use_last_error=True)
u32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
EnumWindows = u32.EnumWindows
EnumWindowsProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
TMP = "C:/Users/Admin/AppData/Local/Temp/"
EXE = r"D:\_APP_CODE\ImageEdit\target\debug\imageedit.exe"
env = dict(os.environ)
env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")


def cursor():
    pt = wt.POINT()
    u32.GetCursorPos(ctypes.byref(pt))
    return pt.x, pt.y


def windows_of_pid(pid):
    found = []

    def cb(hwnd, _):
        p = wt.DWORD()
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and u32.IsWindowVisible(hwnd):
            r = wt.RECT()
            u32.GetWindowRect(hwnd, ctypes.byref(r))
            found.append((hwnd, r.left, r.top, r.right - r.left, r.bottom - r.top))
        return True

    EnumWindows(EnumWindowsProc(cb), 0)
    return found


def report(pid, expect, tag):
    seen = []
    for hwnd, x, y, w, h in windows_of_pid(pid):
        c = (x + w // 2, y + h // 2)
        d = (c[0] - expect[0], c[1] - expect[1])
        seen.append((hwnd, w, h, d))
        flag = "OK" if d == (0, 0) else "偏离"
        print(f"  {tag} hwnd={hwnd} {w}x{h} 中心={c} 期望={expect} 偏差={d} [{flag}]")
    return seen


# --- 1) 主窗口：光标先摆到 (1200,500) 再启动
u32.SetCursorPos(1200, 500)
time.sleep(0.2)
spot1 = cursor()
p1 = subprocess.Popen([EXE, TMP + "t1.png"], env=env)
for _ in range(50):
    time.sleep(0.1)
    if windows_of_pid(p1.pid):
        break
time.sleep(0.4)
u32.SetCursorPos(2600, 1100)  # 用户启动后把鼠标挪走
time.sleep(1.2)
print(f"[主窗口] 启动时光标={spot1}")
report(p1.pid, spot1, "主窗口")

# --- 2) 转发打开第二、三张
for i, (name, spot) in enumerate([("t2.png", (2000, 700)), ("t3.png", (900, 400))]):
    u32.SetCursorPos(*spot)
    time.sleep(0.25)
    expect = cursor()
    p2 = subprocess.Popen([EXE, TMP + name], env=env)
    try:
        p2.wait(timeout=8)
    except subprocess.TimeoutExpired:
        p2.kill()
    time.sleep(1.8)
    print(f"[第 {i + 2} 张] 双击时光标={expect}")
    report(p1.pid, expect, f"第 {i + 2} 张")

p1.terminate()
