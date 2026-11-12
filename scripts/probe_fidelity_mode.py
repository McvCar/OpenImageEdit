"""保真模式的无人工回归探针（不需要点击、不需要 SendInput）。

启动真实 UI 实例，读 stdout 里的结构化事件，然后按模式断言：

    default     打开就是合并图，全程不解析图层、不锁编辑
    enter       保真 -> 阶段 2 解析 -> 停在分层态
    roundtrip   装配完立刻换回保真
    roundtrip2  再进一次分层，走缓存路径（不重读文件）

    python scripts/probe_fidelity_mode.py [--seconds 12] [--only default]

实现见 docs/PSD分层可编辑-B方案.md §16。
"""

import argparse
import json
import os
import subprocess
import sys
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
PSD = "D:/_APP_CODE/_XhCompany/FontGame/Res/0302-界面UI-1.psd"
QT_BIN = "C:/Qt/6.8.3/msvc2022_64/bin"


def collect(exe, psd, mode, seconds):
    env = os.environ.copy()
    env.pop("IMAGEEDIT_AUTO_LAYERED", None)
    if mode == "default":
        env.pop("IMAGEEDIT_FIDELITY_PROBE", None)
    else:
        env["IMAGEEDIT_FIDELITY_PROBE"] = mode
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    proc = subprocess.Popen(
        [exe, psd],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=env,
    )
    lines = []
    reader = threading.Thread(
        target=lambda: lines.extend(proc.stdout.readlines()), daemon=True
    )
    reader.start()
    time.sleep(seconds)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
    reader.join(timeout=2)
    events = []
    for line in lines:
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
    return events


def names(events, event):
    return [e for e in events if e.get("event") == event]


def check(mode, events):
    problems = []
    if not any(e.get("event") == "mark" and e.get("name") == "first-frame"
               for e in events):
        problems.append("首帧没有出现")
    state = names(events, "fidelity-state")
    if not state or not state[0].get("value"):
        problems.append("打开时没有进入保真模式")
    modes = [e.get("value") for e in names(events, "fidelity-mode")]
    enters = [e.get("mode") for e in names(events, "fidelity-enter")]

    if mode == "default":
        if names(events, "editing-locked"):
            problems.append("保真打开竟然锁了编辑工具")
        if names(events, "psd-stage2-ms") or names(events, "psd-layer-progress"):
            problems.append("保真打开竟然启动/推进了阶段 2")
        if modes:
            problems.append("默认路径不应发生显示模式切换")
    elif mode == "enter":
        if "parse" not in enters:
            problems.append("没有按需启动阶段 2（fidelity-enter mode=parse）")
        if not names(events, "psd-stage2-ms"):
            problems.append("阶段 2 没有完成")
        if not modes or modes[-1] is not False:
            problems.append("最终没有停在分层态")
        if not any(e.get("reason") == "done" for e in names(events, "editing-unlocked")):
            problems.append("装配完成后没有解锁编辑")
    elif mode == "roundtrip":
        if modes != [False, True] and modes[-2:] != [False, True]:
            problems.append(f"期望 false→true 往返，实际 {modes}")
        if not any(e.get("reason") == "done" for e in names(events, "editing-unlocked")):
            problems.append("装配完成后没有解锁编辑")
    elif mode == "roundtrip2":
        if "cache" not in enters:
            problems.append("第二次进入分层没有走缓存路径")
        if not modes or modes[-1] is not False:
            problems.append("最终没有停在分层态")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=EXE)
    ap.add_argument("--psd", default=PSD)
    ap.add_argument("--seconds", type=float, default=13.0)
    ap.add_argument("--only", choices=["default", "enter", "roundtrip", "roundtrip2"])
    args = ap.parse_args()
    if not os.path.isfile(args.exe):
        print(f"FAIL: exe 不存在 {args.exe}")
        return 1
    if not os.path.isfile(args.psd):
        print(f"FAIL: PSD 不存在 {args.psd}")
        return 1
    modes = [args.only] if args.only else [
        "default", "enter", "roundtrip", "roundtrip2"
    ]
    failed = 0
    for mode in modes:
        events = collect(args.exe, args.psd, mode, args.seconds)
        problems = check(mode, events)
        tag = "PASS" if not problems else "FAIL"
        print(f"[{tag}] {mode:<11} events={len(events):<4} "
              f"mode-seq={[e.get('value') for e in names(events, 'fidelity-mode')]} "
              f"enter={[e.get('mode') for e in names(events, 'fidelity-enter')]}")
        for p in problems:
            print(f"        - {p}")
        failed += bool(problems)
    print("ALL PASS" if not failed else f"{failed} MODE(S) FAILED")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
