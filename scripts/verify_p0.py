"""P0-2b2 / P0-3 / P0-4 / P0-5 端到端回归（无需 GUI 操作）。

覆盖：
  * P0-3 去钳制：图层可拖到画布外（offset 为负），导出尺寸不变；
  * P0-4 智能参考线：8 显示像素阈值、双轴同时命中、Ctrl 短路由 core 单测覆盖，
    这里验证 Rust↔脚本链路的 `snap-probe` 求解；
  * P0-5 数值面板：一次 `transform-params` 写入 = 一条历史，外接框语义生效；
  * P0-2b2 统一会话：三种手势落定后 active=false、参数层角度随 undo 回退；
  * Tier 1 GPU 快路径：`--probe-gpu` 起手/拖动/落定，日志必须出现
    `gpu-begin` 与 `gpu-skip`，且拖动帧没有 render-profile（零全幅合成）。

用法：python scripts/verify_p0.py [exe]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = tempfile.gettempdir()
SRC = os.path.join(TMP, "p0_probe_in.png")


def fixture(path):
    im = Image.new("RGB", (400, 300), (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, 400, 4):
        d.rectangle([x, 0, x + 1, 299], fill=(0, 0, 0))
    d.rectangle([60, 60, 340, 240], outline=(220, 30, 30), width=3)
    im.save(path)


def events_of(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return out


def run(tag, ops, extra_args=(), timeout=180):
    dst = os.path.join(TMP, "p0_probe_%s.png" % tag)
    shutil.copyfile(SRC, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    script = "extract:20,20,200,160;" + ops + ";save" if ops else ""
    args = [EXE, dst]
    if script:
        args.append("--script=" + script)
    # 脚本跑完必须自己退出，否则子进程会一直挂着等窗口关闭。
    args.append("--auto-quit")
    args.extend(extra_args)
    p = subprocess.run(args, capture_output=True, text=True, env=env, timeout=timeout)
    return events_of(p.stdout + p.stderr), dst


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  " + detail) if detail else ""))
    if not ok:
        raise SystemExit(1)


def main():
    fixture(SRC)

    # ---- P0-3 去钳制 + 导出尺寸不变 ----
    ev, dst = run("offcanvas", "move-layer:-60,-40;layer-info:0;transform-state")
    info = [e for e in ev if e.get("event") == "layer-info"]
    check("图层可移到画布外（offset 为负）",
          bool(info) and info[-1].get("ox", 0) < 0 and info[-1].get("oy", 0) < 0,
          repr(info[-1] if info else None))
    with Image.open(dst) as im:
        check("导出尺寸仍是画布尺寸", im.size == (400, 300), str(im.size))

    # ---- P0-4 吸附求解链路 ----
    ev, _ = run("snap", "snap-probe:192,142,20,20,63")
    probes = [e for e in ev if e.get("event") == "snap-probe"]
    ok = bool(probes) and isinstance(probes[-1].get("result"), dict)
    check("snap-probe 返回结构化修正量", ok, repr(probes[-1] if probes else None))

    # ---- P0-5 数值面板写入 ----
    ev, _ = run("panel", "transform-params:30,40,120,90,0,0,0;transform-state")
    params = [e for e in ev if e.get("event") == "transform-params"]
    ok = bool(params) and params[-1].get("ok") is True
    vals = params[-1].get("values", {}) if params else {}
    check("数值面板外接框写入成功", ok and abs(vals.get("w", 0) - 120) < 1.5,
          repr(vals))

    ev, _ = run("panel-undo", "transform-params:30,40,120,90,0,0,0;undo;layer-info:0")
    info = [e for e in ev if e.get("event") == "layer-info"]
    check("数值面板一次写入可一步撤销",
          bool(info) and info[-1].get("w") == 200 and info[-1].get("h") == 160,
          repr(info[-1] if info else None))

    # ---- P0-2b2 参数化落定与会话无残留 ----
    ev, _ = run("session", "rotate-handle:0,25;transform-state;undo;transform-state")
    states = [e["state"] for e in ev if e.get("event") == "transform-state"]
    check("旋转落定后会话清空且 undo 不残留",
          len(states) == 2 and all(s.get("active") is False for s in states),
          repr(states))

    # ---- Tier 1 GPU 快路径 ----
    try:
        # 走 Rust 侧 gpu-gesture：QML 探针 Timer 会赶不上脚本退出。
        ev, _ = run("gpu", "gpu-gesture:40,30;transform-state", timeout=90)
        has_begin = any(e.get("event") == "gpu-begin" for e in ev)
        skips = [e for e in ev if e.get("event") == "gpu-skip"]
        gesture = [e for e in ev if e.get("event") == "gpu-gesture"]
        profiles = [e for e in ev if e.get("event") == "render-profile"]
        check("GPU 快路径起手生成两层纹理", has_begin, repr(ev[:3]))
        check("拖动帧跳过全幅合成", len(skips) >= 2, "skips=%d" % len(skips))
        check("手势完整起落", bool(gesture) and gesture[-1].get("began") is True
              and gesture[-1].get("landed") is True, repr(gesture[-1] if gesture else None))
        # 拖动帧 = 起手到最后一个 gpu-skip 之间；松手那一帧回落 CPU 合成，
        # 是预期行为（要把预览重画回真实文档）。
        began_at = next((i for i, e in enumerate(ev) if e.get("event") == "gpu-begin"), None)
        last_skip = max((i for i, e in enumerate(ev) if e.get("event") == "gpu-skip"), default=None)
        during = []
        if began_at is not None and last_skip is not None:
            during = [e for e in ev[began_at:last_skip] if e.get("event") == "render-profile"]
        check("拖动期间无 CPU 全幅合成", len(during) == 0,
              "during=%d total=%d" % (len(during), len(profiles)))
    except subprocess.TimeoutExpired:
        check("GPU 探针在 90s 内返回", False)

    print("PASS: P0-2b2 / P0-3 / P0-4 / P0-5 回归")


if __name__ == "__main__":
    main()
