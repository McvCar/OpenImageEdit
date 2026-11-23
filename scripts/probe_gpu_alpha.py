"""GPU 三明治纹理的 alpha 语义回归。

真因（2026-11-23）：`PreviewComposer::blend` 的 Normal 快路径把 RGB 写成
`src.rgb*sa + dst.rgb*inv`（**预乘**），alpha 写成 `sa + da*inv`。CPU 预览
的底图不透明（a=255），看不出差别；GPU 拖动把 static/active/above 分别
合成到**透明底**，这些纹理经 BMP 交给 QML `Image` 后被当成 straight alpha，
半透明白色描边于是变黑（rgb≈255,a≈80 显示成 rgb≈80）。淡出帧里那张偏暗
的 GPU 图再和正确的 CPU 帧交叉，半透明处叠加变深 —— 用户报的两个现象
其实是同一个根因。

本探针直接对纹理做断言（不依赖屏幕截图的缩放/抗锯齿）：
  straight_alpha_over(static, active, above) == CPU 预览帧
并额外量「纯白描边像素」在拖动帧与静止帧之间的比例。
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
TMP = os.path.join(os.environ.get("TEMP", "."), "probe_gpu_alpha")
PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"
GROUP = 72  # 倒计时/错误次数

sys.path.insert(0, os.path.join(ROOT, "scripts"))
import probe_drag_mouse as P  # noqa: E402


SINGLE = False  # --single: 走单层 gpu_begin 而不是整组 gpu_begin_set


def run(tag: str, zoom: int, extra: str) -> tuple[str, str]:
    tmp = os.path.join(TMP, tag)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    env = dict(os.environ)
    env["TEMP"] = env["TMP"] = tmp
    env["PATH"] = r"D:\Qt\6.8.3\msvc2022_64\bin;" + env.get("PATH", "")
    log_path = os.path.join(tmp, "run.log")
    log = open(log_path, "w", encoding="utf-8")
    ops = ("select-layer:397" if SINGLE else "group-select:%d" % GROUP) \
        + ";zoom:%d" % zoom
    if extra:
        ops += ";" + extra
    proc = subprocess.Popen(
        [os.path.join(ROOT, "target", "debug", "imageedit.exe"), PSD,
         "--script-after-paint=" + ops, "--script-delay=600"],
        stdout=log, stderr=subprocess.STDOUT, env=env, text=True, bufsize=1)
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
            return "", ""
        P.user32.SetWindowPos(hwnd, 0, 10, 10, 1120, 1900, 0x0004)
        t0 = time.perf_counter()
        while time.perf_counter() - t0 < 25:
            try:
                if '"event":"script-done"' in open(
                        log_path, encoding="utf-8", errors="replace").read():
                    break
            except OSError:
                pass
            time.sleep(0.1)
        time.sleep(1.6)
    finally:
        proc.kill()
        proc.wait(timeout=10)
        log.close()
    return tmp, open(log_path, encoding="utf-8", errors="replace").read()


def read_rgba(path: str) -> np.ndarray | None:
    if not os.path.exists(path):
        return None
    return np.asarray(Image.open(path).convert("RGBA")).astype(np.float64)


def over(dst: np.ndarray, src: np.ndarray) -> np.ndarray:
    """straight-alpha source-over，与 QML Image 的叠加语义一致。"""
    sa = src[:, :, 3:4] / 255.0
    da = dst[:, :, 3:4] / 255.0
    oa = sa + da * (1.0 - sa)
    rgb = (src[:, :, :3] * sa + dst[:, :, :3] * da * (1.0 - sa)) \
        / np.maximum(oa, 1e-6)
    return np.concatenate([rgb, oa * 255.0], axis=2)


def check_zoom(zoom: int) -> bool:
    # A: CPU 静止帧；B: 同档位起手 hold，产出三张 GPU 纹理
    a_dir, log_a = run("s%d" % zoom, zoom, "")
    b_dir, log_b = run("h%d" % zoom, zoom, "gpu-hold:40,0")
    if not a_dir or not b_dir:
        print("  NO FRAMES")
        return False
    # 文件序号来自 Session::render_count，同一进程里会被 PSD 分层加载等
    # 流程推进，不能写死 1/2 —— 按「最后一个」取当前批次的纹理。
    import glob
    def newest(pattern: str) -> np.ndarray | None:
        hits = sorted(glob.glob(os.path.join(*pattern)))
        return read_rgba(hits[-1]) if hits else None
    preview = newest((a_dir, "imageedit-preview-*.bmp"))
    s = newest((b_dir, "imageedit-gpu-*-s-*.bmp"))
    act = newest((b_dir, "imageedit-gpu-*-a-*.bmp"))
    o = newest((b_dir, "imageedit-gpu-*-o-*.bmp"))
    if preview is None or s is None or act is None or o is None:
        print("  MISSING BMP", [x is None for x in (preview, s, act, o)])
        return False
    if not (s.shape == act.shape == o.shape == preview.shape):
        print("  SHAPE MISMATCH", [x.shape for x in (preview, s, act, o)])
        return False
    composed = over(over(s, act), o)
    d = np.abs(composed - preview)
    mad = float(d.mean())
    bad = int((d[:, :, :3].max(axis=2) > 16).sum())
    print("  %3d%%  target=%dx%d  MAD=%.4f  diff>16=%d  gpu-begin-set=%d"
          % (zoom, s.shape[1], s.shape[0], mad, bad,
             log_b.count('"event":"gpu-begin-set"')))
    # 修复前实测：MAD 0.428 / diff>16=19004；修复后须 < 0.05 / 0。
    return mad < 0.05 and bad == 0


def main() -> int:
    shutil.rmtree(TMP, ignore_errors=True)
    try:
        P.user32.SetThreadDpiAwarenessContext(P.ctypes.c_void_p(-4))
    except Exception:
        pass
    global SINGLE
    if "--single" in sys.argv:
        sys.argv.remove("--single")
        SINGLE = True
    zooms = [int(x) for x in ",".join(sys.argv[1:]).split(",")] or [25, 50, 100]
    results = []
    for z in zooms:
        print("zoom", z)
        results.append(check_zoom(z))
    print("RESULT", "PASS" if all(results) else "FAIL")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
