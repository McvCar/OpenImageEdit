#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""V 键（移动工具开关）回归探针。

用户报「按 v 控制不了移动工具开关」。这条探针把「到底哪一环断了」拆开：

  1. 快捷键本身有没有被派发（按完 V，`--probe-keys` 里的 `moveMode=` 变不变）
  2. `moveShortcut.enabled` 是否为 true（`vEnabled=`）
  3. 窗口是否真的 active（`appActive=` / `winActive=`）—— ApplicationShortcut
     在窗口没激活时压根不会触发，和代码无关
  4. 对照组：M（选区工具）、J（修复组）同样按键，看是 V 单独坏还是整批

用法：
    python scripts/probe_v_key.py
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
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
FIXTURE = os.path.join(ROOT, "probe_v_key.png")
LOG = os.path.join(ROOT, "probe_v_key.log")

u = ctypes.windll.user32
WM_KEYDOWN, WM_KEYUP = 0x0100, 0x0101
VK = {"V": 0x56, "M": 0x4D, "J": 0x4A, "ESC": 0x1B}


def fixture():
    from PIL import Image, ImageDraw
    im = Image.new("RGB", (900, 700), (240, 240, 240))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 899, 699], outline=(200, 60, 60), width=3)
    d.ellipse([260, 220, 620, 560], fill=(70, 130, 200))
    im.save(FIXTURE)


def lp(up):
    return 1 | (1 << 16) | (1 << 24) | (1 << 30) | (1 << 31 if up else 0)


def find_window(pid):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _):
        p = wt.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        if p.value == pid and u.IsWindowVisible(hwnd):
            buf = ctypes.create_unicode_buffer(256)
            u.GetWindowTextW(hwnd, buf, 256)
            if buf.value:
                found.append(hwnd)
        return True

    u.EnumWindows(cb, 0)
    return found[0] if found else None


def key(hwnd, name, shift=False):
    vk = VK[name]
    u.SetForegroundWindow(hwnd)
    time.sleep(0.15)
    if shift:
        u.PostMessageW(hwnd, WM_KEYDOWN, 0x10, lp(False))
    u.PostMessageW(hwnd, WM_KEYDOWN, vk, lp(False))
    time.sleep(0.04)
    u.PostMessageW(hwnd, WM_KEYUP, vk, lp(True))
    if shift:
        u.PostMessageW(hwnd, WM_KEYUP, 0x10, lp(True))


def probes():
    if not os.path.exists(LOG):
        return []
    txt = open(LOG, encoding="utf-8", errors="replace").read()
    return [ln for ln in txt.splitlines() if "keys-probe:" in ln]


def state(line):
    body = line.split("keys-probe:", 1)[1]
    return dict(re.findall(r"(\w+)=([^\s\"]+)", body))


def wait_new(mark, timeout=4.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        ps = probes()
        if len(ps) > mark:
            return state(ps[-1])
        time.sleep(0.1)
    return None


def main():
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fixture()
    if os.path.exists(LOG):
        os.remove(LOG)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + ";" + env.get("PATH", "")
    log = open(LOG, "w", encoding="utf-8")
    p = subprocess.Popen([EXE, FIXTURE, "--probe-keys", "--no-restore-tool",
                          "--script=extract:260,220,360,340"],
                         stdout=log, stderr=subprocess.STDOUT, env=env, cwd=ROOT)
    fails = []

    def check(name, ok, detail=""):
        print("%-40s %s %s" % (name, "PASS" if ok else "FAIL", detail))
        if not ok:
            fails.append(name)

    try:
        time.sleep(3.5)
        hwnd = find_window(p.pid)
        if not hwnd:
            print("FAIL 没有窗口")
            return 1
        base = wait_new(0)
        if base is None:
            print("FAIL 没读到 keys-probe")
            return 1
        print("基线：", {k: base.get(k) for k in
                        ("appActive", "winActive", "ready", "vEnabled",
                         "canUse", "moveMode", "selTool", "paintTool", "focusItem")})

        steps = [("V", False, "moveMode", "v")]
        for kname, shift, field, short in steps:
            m0 = len(probes())
            key(hwnd, kname, shift)
            st = wait_new(m0)
            print("  按 %-6s → %s=%s vEnabled=%s appActive=%s"
                  % (kname, field, st.get(field) if st else "?",
                     st.get("vEnabled") if st else "?", st.get("appActive") if st else "?"))
            check("按 V 后 moveMode 翻转",
                  st is not None and st.get(field) != base.get(field),
                  "%s → %s" % (base.get(field), st.get(field) if st else "?"))
            # 再按一次应回到原值（真·开关，不是单次）
            m0 = len(probes())
            key(hwnd, kname, shift)
            st2 = wait_new(m0)
            check("再按 V 后 moveMode 复位",
                  st2 is not None and st2.get(field) == base.get(field),
                  "%s" % (st2.get(field) if st2 else "?"))

        # 对照：M / J
        for kname, field, want in (("M", "selTool", "rect"), ("J", "paintTool", "spot")):
            m0 = len(probes())
            key(hwnd, kname)
            st = wait_new(m0)
            print("  按 %-6s → %s=%s" % (kname, field, st.get(field) if st else "?"))
            check("对照键 %s 生效" % kname,
                  st is not None and st.get(field) == want,
                  "%s" % (st.get(field) if st else "?"))

        print()
        if fails:
            print("RESULT FAIL:", ", ".join(fails))
            return 1
        print("RESULT ALL PASS")
        return 0
    finally:
        try:
            p.terminate()
        except Exception:
            pass
        log.close()


if __name__ == "__main__":
    sys.exit(main())
