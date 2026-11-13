#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""PSD 图层组探针：打开带嵌套组的 PSD，检查组树是否被还原。

覆盖：
  1. `groups-dump`：组的父子关系（p）/ 深度（d）/ 折叠态（c）/ 子树成员数；
  2. `layers-dump`：每层的 `g` 指向正确的组；
  3. 面板显示行：折叠的父组把整棵子树的成员行都收起来，展开后行数回来；
  4. 整组显隐：`group-visible` 一次盖住嵌套成员；
  5. 保存语义：组不会写进 PSD（层记录数 = 图层数），`psd-saved` 如实报分组数。

跑法：
    python scripts/probe_psd_groups.py
输出：
    PASS / FAILED，逐条打印检查点。
"""
import os
import shutil
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

FIXTURE = os.path.join(ROOT, "tmp", "psd-fixtures", "nested-groups.psd")
WORK_DIR = os.path.join(ROOT, "tmp", "psd-groups")

HOOKS = ("IMAGEEDIT_PSD_AUTO_SKIP", "IMAGEEDIT_PSD_STAGE2_FAIL")


def env():
    e = dict(os.environ)
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    e["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + e.get("PATH", "")
    for key in HOOKS:
        e.pop(key, None)
    return e


def run(path, script, needle, timeout=45.0, extra=None):
    import subprocess
    import time

    args = [EXE, path]
    if script:
        args.append("--script=" + script)
    if extra:
        args.extend(extra)
    proc = subprocess.Popen(
        args,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, errors="replace", env=env(), bufsize=1,
    )
    lines, _ = pump(proc, needle, timeout=timeout)
    time.sleep(0.4)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()
    return lines


def fatal(lines):
    return any(
        ("qml-load-failed" in l) or ("is not a type" in l) or ("TypeError" in l)
        for l in lines
    )


def groups(lines):
    dumps = parse_events(lines, "groups-dump")
    return dumps[-1].get("groups", []) if dumps else []


def rows(lines):
    dumps = parse_events(lines, "layers-dump")
    return dumps[-1].get("layers", []) if dumps else []


def panel_rows(lines):
    dumps = parse_events(lines, "panel-rows-dump")
    return dumps[-1].get("rows", []) if dumps else []


def layer_records(path):
    """独立统计 PSD 里的层记录数（用来断言组没有被写成假图层）。"""
    b = open(path, "rb").read()
    o = 26
    o += 4 + struct.unpack_from(">I", b, o)[0]
    o += 4 + struct.unpack_from(">I", b, o)[0]
    o += 4
    o += 4
    return abs(struct.unpack_from(">h", b, o)[0])


def main():
    if not os.path.exists(EXE):
        print("FAIL debug exe 不存在:", EXE)
        return 1
    if not os.path.exists(FIXTURE):
        print("FAIL fixture 不存在，先跑 make_psd_groups_fixture.py")
        return 1
    problems = []

    print("--- 1) 组树还原 ---", flush=True)
    # 这条进程同时带 --probe-panel-rows：等它把显示行也吐出来再收尾，
    # 否则 0.4 s 的 settle 会把 900 ms 的定时器掐掉（第一版就是这么空手的）。
    lines = run(FIXTURE, "groups-dump;layers-dump", '"event":"panel-rows-dump"',
                extra=["--probe-panel-rows"])
    if fatal(lines):
        problems.append("QML 加载报错")
    gs = {g["n"]: g for g in groups(lines)}
    print("  groups:", [(g["n"], g["p"], g["d"], g["c"], g["cnt"]) for g in gs.values()])
    if set(gs) != {"Button", "Outer", "Inner"}:
        problems.append("组名不符：%s" % list(gs))
    else:
        if gs["Outer"]["d"] != 0 or gs["Inner"]["d"] != 1:
            problems.append("嵌套深度不对：Outer.d=%s Inner.d=%s"
                            % (gs["Outer"]["d"], gs["Inner"]["d"]))
        if gs["Inner"]["p"] != gs["Outer"]["id"]:
            problems.append("Inner 的父组不是 Outer")
        if not gs["Outer"]["c"]:
            problems.append("文件里存成关闭态的组没被识别为折叠")
        if gs["Outer"]["cnt"] != 3:
            problems.append("父组子树成员数 %s，应为 3（含嵌套组两层）"
                            % gs["Outer"]["cnt"])

    print("--- 2) 层归属 ---", flush=True)
    rs = rows(lines)
    by_name = {r["n"]: r for r in rs}
    print("  rows:", [(r["n"], r.get("g")) for r in rs])
    expect = {
        "Inner A": gs.get("Inner", {}).get("id"),
        "Inner B": gs.get("Inner", {}).get("id"),
        "Outer Loose": gs.get("Outer", {}).get("id"),
        "Btn Fill": gs.get("Button", {}).get("id"),
        "Btn Text": gs.get("Button", {}).get("id"),
        "Background": 0,
    }
    for name, want in expect.items():
        got = by_name.get(name, {}).get("g")
        if got != want:
            problems.append("%s 的组 id %s，应为 %s" % (name, got, want))

    print("--- 3) 折叠/展开的面板行数 ---", flush=True)
    # run 1 已经带着 --probe-panel-rows 跑完（Outer 在文件里就是关闭态）。
    collapsed = panel_rows(lines)
    lines2 = run(FIXTURE, "group-collapse:%d,0"
                 % gs.get("Outer", {}).get("id", 0), '"event":"panel-rows-dump"',
                 extra=["--probe-panel-rows"])
    expanded = panel_rows(lines2)
    if fatal(lines2):
        problems.append("QML 加载报错（展开轮）")
    print("  collapsed rows=%d expanded rows=%d" % (len(collapsed), len(expanded)))
    if not collapsed:
        problems.append("拿不到面板显示行（panel-rows-dump 为空）")
    else:
        names = [r.get("n") for r in collapsed]
        if any(n in ("Inner", "Inner A", "Inner B") for n in names):
            problems.append("折叠的父组仍然露出子组/子层：%s" % names)
        if len(expanded) - len(collapsed) != 4:
            # Outer 子树共 4 行：Outer Loose + Inner 标题 + Inner A/B
            problems.append("展开后行数只多了 %d 行，应为 4"
                            % (len(expanded) - len(collapsed)))

    print("--- 4) 整组显隐覆盖嵌套 ---", flush=True)
    lines3 = run(FIXTURE, "group-visible:%d,0;layers-dump"
                 % gs.get("Outer", {}).get("id", 0), '"event":"layers-dump"')
    hidden = {r["n"]: r["v"] for r in rows(lines3)}
    print("  visible after hide:", hidden)
    for name in ("Outer Loose", "Inner A", "Inner B"):
        if hidden.get(name) is not False:
            problems.append("整组隐藏没有盖住嵌套成员 %s" % name)
    if hidden.get("Btn Fill") is not True:
        problems.append("隐藏一个组把别的组也关了")

    print("--- 5) 另存为 PSD：分组要写进文件 ---", flush=True)
    os.makedirs(WORK_DIR, exist_ok=True)
    dst = os.path.join(WORK_DIR, "groups-export.psd")
    if os.path.exists(dst):
        os.remove(dst)
    lines4 = run(FIXTURE, "save-as:" + dst.replace("\\", "/"), '"event":"saved"')
    if fatal(lines4):
        problems.append("QML 加载报错（导出轮）")
    if not parse_events(lines4, "saved"):
        problems.append("另存为 PSD 没有 saved 事件")
    saved = parse_events(lines4, "psd-saved")
    if not saved:
        problems.append("缺 psd-saved 事件")
    elif saved[-1].get("groups", 0) < 3:
        problems.append("psd-saved 没报出 3 个分组（groups=%s）"
                        % saved[-1].get("groups"))
    if not os.path.exists(dst):
        problems.append("导出的 PSD 不存在")
        for p in problems:
            print("FAIL " + p)
        return 1
    # 6 个图层 + 3 个组 × 2 条分隔记录（组底边界 + 文件夹行）。
    n = layer_records(dst)
    print("  exported records=%d" % n)
    if n != 12:
        problems.append("导出记录 %d 条，应为 12（6 层 + 3 组的分隔记录）" % n)

    # 再开一次：组树必须原样回来（名称 / 父子 / 折叠态）。
    lines5 = run(dst, "groups-dump", '"event":"groups-dump"')
    if fatal(lines5):
        problems.append("QML 加载报错（重开轮）")
    gs2 = {g["n"]: g for g in groups(lines5)}
    print("  reopened groups:", [(g["n"], g["p"], g["d"], g["c"]) for g in gs2.values()])
    if set(gs2) != {"Button", "Outer", "Inner"}:
        problems.append("重开后组名不对：%s" % list(gs2))
    else:
        if gs2["Inner"]["p"] != gs2["Outer"]["id"]:
            problems.append("重开后 Inner 的父组不是 Outer")
        if not gs2["Outer"]["c"]:
            problems.append("重开后 Outer 的折叠态丢了")

    if problems:
        for p in problems:
            print("FAIL " + p)
        return 1
    print("ALL PSD GROUP CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
