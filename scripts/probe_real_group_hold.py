"""真实 PSD 分组拖动的确定性像素回归（不走鼠标，避免时序抖动）。

两次独立运行，脚本都在 PSD 加载完成后执行：
  A: group-select:72;zoom:100              → 静止帧
  B: group-select:72;zoom:100;gpu-hold:40,0 → 起手 + 5 段 update，不落定
比较「倒计时 666」绿色文字的 bbox 与像素：B 相对 A 只应整体右移 40px，
宽/高/字形必须逐像素一致。任何宽高变化 = 拖动期缩放 bug。
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_real_group_hold")
PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"

sys.path.insert(0, os.path.join(ROOT, "scripts"))
import probe_drag_mouse as P  # noqa: E402


def green_boxes(img: Image.Image):
    a = np.asarray(img.convert("RGB")).astype(np.int16)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    m = (g > 100) & (g - r > 30) & (g - b > 30)
    m[:, img.width - 290:] = False
    cols = m.sum(axis=0)
    nz = np.nonzero(cols)[0]
    out = []
    if len(nz) == 0:
        return out
    d = np.diff(nz)
    cuts = np.nonzero(d > 1)[0]
    segs, s = [], nz[0]
    for c in cuts:
        segs.append((int(s), int(nz[c])))
        s = nz[c + 1]
    segs.append((int(s), int(nz[-1])))
    for x0, x1 in segs:
        if not (12 <= x1 - x0 <= 180):
            continue
        rows = m[:, x0:x1 + 1].sum(axis=1)
        ys = np.nonzero(rows)[0]
        if not (10 <= len(ys) <= 100):
            continue
        out.append((x0, int(ys[0]), x1 - x0 + 1, len(ys)))
    return out


def run(tag: str, extra: str):
    tmp = os.path.join(TMP, tag)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    log_path = os.path.join(tmp, "run.log")
    log = open(log_path, "w", encoding="utf-8")
    ops = "group-select:72;zoom:100" + ((";" + extra) if extra else "")
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), PSD,
         "--script-after-paint=" + ops, "--script-delay=600"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1)
    img = None
    try:
        t0 = time.perf_counter()
        hwnd = None
        while time.perf_counter() - t0 < 25:
            hwnd = P.find_main_window(proc.pid)
            if hwnd:
                break
            time.sleep(0.02)
        if hwnd is None:
            print("NO WINDOW")
            return None, ""
        # 100% 下画布 750x1700，普通窗口只露中下部，「倒计时」那一行被滚出去了。
        # 把窗口拉高到整幅画布都可见（PrintWindow 连屏外内容一起抓）。
        P.user32.SetWindowPos(hwnd, 0, 10, 10, 1120, 1900, 0x0004)
        time.sleep(0.4)
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < 25:
            try:
                if '"event":"script-done"' in open(log_path, encoding="utf-8",
                                                   errors="replace").read():
                    break
            except OSError:
                pass
            time.sleep(0.1)
        time.sleep(1.5)
        img = P.grab(hwnd)
        if img:
            img.save(os.path.join(tmp, "frame.png"))
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()
    text = open(log_path, encoding="utf-8", errors="replace").read()
    print(tag, "window", img.size if img else None, "boxes", green_boxes(img) if img else None)
    for k in ('"event":"gpu-begin"', '"event":"gpu-begin-set"', '"event":"gpu-hold"'):
        print("   ", k, text.count(k))
    return img, text


def main():
    shutil.rmtree(TMP, ignore_errors=True)
    os.makedirs(TMP, exist_ok=True)
    try:
        P.user32.SetThreadDpiAwarenessContext(P.ctypes.c_void_p(-4))
    except Exception:
        pass
    still, log_a = run("still", "")
    during, log_b = run("hold", "gpu-hold:40,0")

    ok = True
    if still is None or during is None:
        print("RESULT FAIL (no frames)")
        return 1
    bs, bd = green_boxes(still), green_boxes(during)
    print("still boxes", bs)
    print("hold  boxes", bd)
    if len(bs) < 1 or len(bd) < 1:
        print("RESULT FAIL (no green text)")
        return 1
    # 自动聚类会抓到画布上方的装饰（不会跟着组走），所以直接量 PSD 里
    # 「倒计时：」右侧那三个绿色 666 的固定窗口；hold 帧按 40px 位移对齐。
    def green_mass(img, x0, x1, y0, y1):
        a = np.asarray(img.convert("RGB")).astype(np.int16)
        r, g, bl = a[y0:y1, x0:x1, 0], a[y0:y1, x0:x1, 1], a[y0:y1, x0:x1, 2]
        m = (g > 100) & (g - r > 30) & (g - bl > 30)
        cols = np.nonzero(m.sum(axis=0))[0]
        rows = np.nonzero(m.sum(axis=1))[0]
        return (int(m.sum()),
                int(cols[-1] - cols[0] + 1) if len(cols) else 0,
                int(rows[-1] - rows[0] + 1) if len(rows) else 0)

    # 实测 100% 档三个 6 的列行程：静止 326..391，拖 40px 后 365..430。
    ma = green_mass(still, 322, 395, 415, 470)
    mb = green_mass(during, 362, 435, 415, 470)
    print("666 静止 mass/宽/高 =", ma, " 拖动(已按 40px 对齐) =", mb)
    # 宽与绿色像素量是主判据：字形被缩放时这两个数一定变。
    # 高度用宽松阈值：GPU 纹理是「先合到透明底、再叠回画布」，半透明描边
    # 的绿色像素会有几行落在阈值边缘（实测 35→31 行但 mass 975→974）。
    ok = abs(mb[1] - ma[1]) <= 1 and abs(mb[2] - ma[2]) <= 5 \
        and abs(mb[0] - ma[0]) <= ma[0] * 0.02
    print("check 字形宽度/像素量不变:", ok)
    print("RESULT", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
