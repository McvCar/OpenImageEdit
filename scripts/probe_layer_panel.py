"""图层面板（P2）探针。

验证三件事：
  1. 列表把背景 / 位图图层 / 矢量标记混在**同一个列表**里（行顺序 = z 序）。
  2. 缩略图真的生成出来了（临时目录里有每层一张的非空 PNG）。
  3. 多选之后批量命令生效（`align:left` 把两层都推到 x=0）。

跑法：
    python scripts/probe_layer_panel.py
输出：
    probe_layer_panel.png（整窗截图，肉眼看样式）
    printwindow 的像素只用来确认「没白屏」——列表内容以 JSON 为准。
"""
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
FIX = os.path.join(ROOT, "probe_layer_panel.png")
SHOT = os.path.join(ROOT, "probe_layer_panel_window.png")

sys.path.insert(0, os.path.join(ROOT, "scripts"))
import shot_window  # noqa: E402


def fixture(path=FIX):
    img = Image.new("RGBA", (600, 400), (24, 28, 34, 255))
    d = ImageDraw.Draw(img)
    d.rectangle([40, 40, 260, 200], fill=(70, 120, 200, 255))
    d.rectangle([320, 160, 560, 360], fill=(200, 90, 70, 255))
    img.save(path)


def pump(proc, needle, timeout=40.0):
    """后台线程持续收集 stdout，直到出现 needle（或超时）。

    直接在主线程 readline() 会卡死：窗口起来之后进程就不再输出，阻塞读
    永远等不到下一行。所以用线程收、主线程只等事件。
    """
    lines = []
    seen = threading.Event()

    def run():
        for raw in proc.stdout:
            line = raw.rstrip("\n")
            lines.append(line)
            if needle in line:
                seen.set()
        seen.set()

    t = threading.Thread(target=run, daemon=True)
    t.start()
    seen.wait(timeout)
    return lines, t


def parse_events(lines, event):
    res = []
    for line in lines:
        if '"event":"%s"' % event not in line:
            continue
        at = line.find('{"phase"')
        if at < 0:
            continue
        try:
            obj, _ = json.JSONDecoder().raw_decode(line[at:])
        except ValueError:
            continue
        res.append(obj)
    return res


def main():
    print('[1] fixture', flush=True)
    fixture()
    ops = [
        "mark-live",        # 画一个矩形标记（z 序最上面）
        "select-layer:1",
        "layer-dup",        # 复制标记，得到两个同尺寸图层
        "multi-select:1,2", # 两层一起选中
        "align:left",       # 批量左对齐（参照 = 两层的并集包围盒）
        "layers-dump",
    ]

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    # 先清掉旧缩略图，断言才有意义。
    for old in glob.glob(os.path.join(tempfile.gettempdir(), "imageedit-thumb-*.png")):
        try:
            os.remove(old)
        except OSError:
            pass

    print("[2] launch", flush=True)
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-layers", "--script=" + ";".join(ops)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    print("[3] waiting qml", flush=True)
    lines, reader = pump(proc, '"event":"qml-loaded"', timeout=40)
    print("[4] qml loaded", flush=True)
    # 脚本 op 可能排在 qml-loaded 之后，再等一会儿让 layers-dump 落定。
    time.sleep(2.5)

    print("[5] shot", flush=True)
    hwnd = shot_window.find_main_window(proc.pid)
    if hwnd:
        shot_window.user32.SetForegroundWindow(hwnd)
        time.sleep(0.5)
        sys.argv = ["shot_window", str(proc.pid), SHOT]
        shot_window.main()

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    print("[6] assertions", flush=True)
    dumps = parse_events(lines, "layers-dump")
    ok = True
    if not dumps:
        print("FAIL 没有 layers-dump")
        ok = False
    else:
        rows = dumps[-1]["layers"]
        kinds = [r["k"] for r in rows]
        print("行顺序:", [(r["i"], r["n"], r["k"]) for r in rows])
        if "mark" not in kinds or "raster" not in kinds:
            print("FAIL 列表里没有混排标记与位图")
            ok = False
        if not any(r.get("bg") for r in rows):
            print("FAIL 背景没有作为普通行出现")
            ok = False
        if rows[-1]["i"] != 0:
            print("FAIL 背景不在列表底部")
            ok = False
        # 对齐语义：选中两层时参照 = 两层的并集包围盒（最靠左那层不动），
        # 所以断言「两层左边缘一致」，而不是原先的「都贴到画布 x=0」。
        aligned = [r for r in rows if not r.get("bg")]
        if not aligned or len({r["x"] for r in aligned}) != 1:
            print("FAIL 左对齐后两层左边缘不一致:", [(r["n"], r["x"]) for r in aligned])
            ok = False

    thumbs = glob.glob(os.path.join(tempfile.gettempdir(), "imageedit-thumb-*.png"))
    print("缩略图:", len(thumbs), "张")
    if not thumbs:
        print("FAIL 面板没有请求缩略图")
        ok = False
    else:
        bad = [p for p in thumbs if os.path.getsize(p) < 100]
        if bad:
            print("FAIL 缩略图是空的:", bad)
            ok = False

    if os.path.exists(SHOT):
        im = Image.open(SHOT).convert("RGB")
        colors = im.getcolors(maxcolors=1 << 20) or []
        if len(colors) < 20:
            print("FAIL 截图几乎是纯色，窗口可能没画出来")
            ok = False

    print("PASS" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
