#!/usr/bin/env python
"""按键链路探针：向指定 PID 的窗口投递真实 WM_KEYDOWN，验证快捷键是否生效。

用途：用户报「所有快捷键失效」时，区分三种情况 ——
  1. 按键根本没到窗口（投递了但 Shortcut 不响应，且 moveMode 不变）
  2. 绑定被禁用（enabled=false，端上探针能直接看到）
  3. 焦点/多窗口问题（SetForegroundWindow 前后行为不同）

为什么不用 PostMessage 到非前台窗口：Qt 的 Shortcut 走
QWindowSystemInterface，非前台窗口的 WM_KEYDOWN 会被键盘映射层丢弃，
所以这里先 SetForegroundWindow，再投递。

用法：
    python scripts/probe_keys_live.py --pid 1234 --key V --wait 0.6
    python scripts/probe_keys_live.py --pid 1234 --keys V,F11,Ctrl+Z
环境变量 IMAGEEDIT_KEY_LOG 指向实例日志，脚本会读 moveMode / toast 判定。
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time

user32 = ctypes.WinDLL("user32", use_last_error=True)
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101
WM_CHAR = 0x0102

VK = {
    "V": 0x56, "M": 0x4D, "L": 0x4C, "W": 0x57, "J": 0x4A, "G": 0x47, "Q": 0x51,
    "F11": 0x7A, "F12": 0x7B, "ESC": 0x1B, "CTRL": 0x11, "SHIFT": 0x10,
    "Z": 0x5A, "S": 0x53, "O": 0x4F, "0": 0x30, "9": 0x39, "DEL": 0x2E,
}

user32.EnumWindows.argtypes = [ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM), wt.LPARAM]
user32.EnumWindows.restype = wt.BOOL
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.GetWindowTextLengthW.argtypes = [wt.HWND]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.MapVirtualKeyW.argtypes = [wt.UINT, wt.UINT]


def windows_of(pid: int):
    out = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _lp):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(hwnd) and user32.GetWindowTextLengthW(hwnd) > 0:
            out.append(hwnd)
        return True

    user32.EnumWindows(cb, 0)
    return out


def parse_seq(seq: str):
    """'Ctrl+Shift+Z' → (mods, vk)。mods 只用于记录，投递时逐个按下。"""
    parts = [p.strip() for p in seq.split("+") if p.strip()]
    mods, key = parts[:-1], parts[-1]
    return mods, key.upper()


def press(hwnd, seq: str, hold_ms: int = 40):
    mods, key = parse_seq(seq)
    vk = VK.get(key)
    if vk is None:
        raise SystemExit("未知按键 %r（在 VK 表里补一下）" % key)
    mod_vks = [VK[m.upper()] for m in mods if m.upper() in VK]
    sc = user32.MapVirtualKeyW(vk, 0)

    def lparam(up: bool):
        v = 1 | (sc << 16)
        if up:
            v |= (1 << 30) | (1 << 31)
        return ctypes.c_long(v).value

    for mv in mod_vks:
        user32.PostMessageW(hwnd, WM_KEYDOWN, mv, lparam(False))
    user32.PostMessageW(hwnd, WM_KEYDOWN, vk, lparam(False))
    time.sleep(hold_ms / 1000.0)
    user32.PostMessageW(hwnd, WM_KEYUP, vk, lparam(True))
    for mv in reversed(mod_vks):
        user32.PostMessageW(hwnd, WM_KEYUP, mv, lparam(True))


def tail(path: str, n: int = 60) -> str:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return "".join(f.readlines()[-n:])
    except OSError:
        return ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--keys", default="V")
    ap.add_argument("--wait", type=float, default=0.8, help="每次投递后等待秒数")
    ap.add_argument("--log", default=os.environ.get("IMAGEEDIT_KEY_LOG", ""))
    a = ap.parse_args()

    hwnds = windows_of(a.pid)
    if not hwnds:
        print("FAIL 找不到 PID %d 的可见窗口" % a.pid)
        return 1
    hwnd = hwnds[0]
    print("PID=%d 窗口数=%d hwnd=%#x" % (a.pid, len(hwnds), hwnd))
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.25)

    ok = True
    for seq in a.keys.split(","):
        before = tail(a.log) if a.log else ""
        press(hwnd, seq)
        time.sleep(a.wait)
        after = tail(a.log) if a.log else ""
        got = after
        # 判定：日志尾部出现新的 keys-probe 行且 moveMode 变了，或出现 toast。
        changed = got != before
        probe = [ln for ln in got.splitlines() if "keys-probe" in ln]
        last = probe[-1] if probe else "(无 keys-probe 行)"
        print("  %-8s 日志新增=%s  最后状态：%s" % (seq, changed, last[-150:]))
        if not changed:
            ok = False
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
