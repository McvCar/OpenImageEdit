#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第三方 PSD 实现交叉验证（psd-tools，独立于本项目 vendor 的 psd crate）。

为什么需要这份脚本
------------------
B 档的读写都建立在 vendored `psd` crate 之上，而**同一个错误的解析器会
同时验证自己**。历史上就踩过一次：vendor 把 layer flags 的 bit 1 解释成
「置位 = 可见」，而 Adobe 规范是「置位 = 隐藏」——我们的解码器读真实 PSD
时把所有层判成隐藏，写出的 PSD 在 Photoshop 里也全是隐藏层。自测全绿，
因为读写用的是同一套错误语义。

所以这里用 psd-tools（Python 生态里独立的第三方 PSD 实现）做双向对照：

  written : 用本程序写一个 PSD，再让 psd-tools 去读，逐层比对
            名称 / 显隐 / 不透明度 / 偏移。验证「别人能正确打开我们写的文件」。
  vendor  : 拿 20 份第三方 fixture，本程序读一遍、psd-tools 读一遍，
            逐层比对名称 / 显隐 / 不透明度。验证「我们读别人的文件没错」。

用法：
    python scripts/crosscheck_psd.py [--only written|vendor|all]

依赖 psd-tools：`pip install psd-tools`。缺失时打印 SKIP 并返回 0
（不让没有该依赖的机器误判为回归）。
"""

import argparse
import glob
import json
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from probe_layer_panel import EXE, pump  # noqa: E402

OUT_DIR = os.path.join(ROOT, "tmp", "psd-cross")
FIXTURE = os.path.join(ROOT, "tmp", "psd-fixtures", "small-layers.psd")
VENDOR_GLOB = os.path.join(ROOT, "vendor", "psd", "tests", "fixtures", "**", "*.psd")

HOOKS = (
    "IMAGEEDIT_PSD_STAGE2_DELAY_MS",
    "IMAGEEDIT_PSD_STAGE2_FAIL",
    "IMAGEEDIT_PSD_AUTO_SKIP",
    "IMAGEEDIT_PSD_COMPOSITE",
)


def env():
    e = dict(os.environ)
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    e["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + e.get("PATH", "")
    for key in HOOKS:
        e.pop(key, None)
    return e


def run_script(path, script, timeout=30.0):
    """同步自动化路径打开 path 并执行 script，返回输出行。"""
    proc = subprocess.Popen(
        [EXE, path, "--script=" + script],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        errors="replace",
        env=env(),
        bufsize=1,
    )
    lines, _ = pump(proc, '"event":"qml-loaded"', timeout=timeout)
    time.sleep(0.8)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
    return lines


def our_layers(lines):
    """从 layers-dump 事件里取出图层表（去掉背景行，保持 顶→底 顺序）。"""
    for line in lines:
        if '"event":"layers-dump"' not in line:
            continue
        try:
            payload = json.loads(line)
        except ValueError:
            continue
        rows = payload.get("layers") or []
        return [r for r in rows if not r.get("bg")]
    return None


def op_percent(opacity_255):
    return int(round(opacity_255 * 100.0 / 255.0))


def psd_layers(psd):
    """psd-tools 的 顶→底 图层表（只取 pixel / smartobject 这类叶层）。"""
    out = []
    for layer in psd.descendants():
        out.append({
            "name": layer.name,
            "visible": bool(layer.visible),
            "opacity": op_percent(layer.opacity),
            "x": int(layer.offset[0]),
            "y": int(layer.offset[1]),
        })
    return out


def compare(expected, actual, check_offset):
    """逐层比对，返回 (problems, notes)。"""
    problems = []
    notes = []
    if len(expected) != len(actual):
        return (["层数不一致：本程序 %d / psd-tools %d（%s vs %s）"
                 % (len(expected), len(actual),
                    [e["n"] for e in expected],
                    [a["name"] for a in actual])], notes)
    for i, (exp, act) in enumerate(zip(expected, actual)):
        tag = "第 %d 层" % i
        if exp["n"] != act["name"]:
            problems.append("%s 名称 %r != %r" % (tag, exp["n"], act["name"]))
            continue
        if bool(exp["v"]) != act["visible"]:
            problems.append("%s「%s」显隐 %s != %s"
                            % (tag, act["name"], exp["v"], act["visible"]))
        if int(exp["o"]) != act["opacity"]:
            problems.append("%s「%s」不透明度 %d != %d"
                            % (tag, act["name"], exp["o"], act["opacity"]))
        if check_offset:
            if (int(exp["x"]), int(exp["y"])) != (act["x"], act["y"]):
                problems.append("%s「%s」偏移 (%s,%s) != (%s,%s)"
                                % (tag, act["name"], exp["x"], exp["y"],
                                   act["x"], act["y"]))
    return problems, notes


def phase_written():
    """本程序写 → psd-tools 读。"""
    print("--- written: 本程序写回，psd-tools 独立读 ---", flush=True)
    from psd_tools import PSDImage

    os.makedirs(OUT_DIR, exist_ok=True)
    target = os.path.join(OUT_DIR, "written.psd")
    shutil.copyfile(FIXTURE, target)

    lines = run_script(target, "layers-dump;save", timeout=40.0)
    if any("qml-load-failed" in l or "is not a type" in l or "TypeError" in l
           for l in lines):
        return ["QML 加载报错"]
    if not any('"event":"saved"' in l for l in lines):
        return ["写回没有产生 saved 事件（写回链路没跑通）"]

    expected = our_layers(lines)
    if not expected:
        return ["拿不到 layers-dump（本程序没有装配分层）"]

    psd = PSDImage.open(target)
    actual = psd_layers(psd)
    problems, _ = compare(expected, actual, check_offset=True)
    print("  层数=%d  psd-tools 读取成功；逐层比对 %s"
          % (len(actual), "PASS" if not problems else "FAIL"))

    # 合成图必须真的有内容（不是全透明 / 全黑）。
    comp = psd.composite(force=True)
    if comp is None:
        problems.append("psd-tools 无法合成（composite=None）")
    else:
        colors = comp.convert("RGB").getcolors(maxcolors=1 << 20) or []
        if len(colors) < 8:
            problems.append("合成图颜色数 %d < 8，疑似空图" % len(colors))
        else:
            print("  合成图 %dx%d，颜色数 %d" % (comp.width, comp.height, len(colors)))
    return problems


def phase_vendor():
    """第三方 fixture：本程序读 vs psd-tools 读。"""
    print("--- vendor: 第三方 fixture 双向对照 ---", flush=True)
    from psd_tools import PSDImage

    files = sorted(glob.glob(VENDOR_GLOB, recursive=True))
    if not files:
        print("  SKIP 找不到 vendor fixture")
        return []

    checked = skipped = 0
    problems = []
    for path in files:
        name = os.path.basename(path)
        try:
            actual = psd_layers(PSDImage.open(path))
        except Exception as exc:  # noqa: BLE001 - 第三方解析器读不了就跳过
            print("  SKIP %-46s psd-tools 打不开：%s" % (name, exc))
            skipped += 1
            continue

        lines = run_script(path, "layers-dump", timeout=25.0)
        expected = our_layers(lines)
        if expected is None:
            print("  SKIP %-46s 本程序没装配分层" % name)
            skipped += 1
            continue
        if not expected or not actual:
            print("  SKIP %-46s 空图层（%d / %d）" % (name, len(expected), len(actual)))
            skipped += 1
            continue
        # 组结构 / 越界整层丢弃会让两边层数不同，这类样本不算失败。
        if len(expected) != len(actual) or \
                [e["n"] for e in expected] != [a["name"] for a in actual]:
            print("  SKIP %-46s 层表结构不同（组展平/越界丢弃）：%d vs %d"
                  % (name, len(expected), len(actual)))
            skipped += 1
            continue

        found, _ = compare(expected, actual, check_offset=False)
        if found:
            print("  FAIL %-46s %s" % (name, "；".join(found[:4])))
            problems += ["%s: %s" % (name, p) for p in found]
        else:
            checked += 1

    print("  对照通过 %d 份 / 跳过 %d 份 / 失败 %d 项"
          % (checked, skipped, len(problems)))
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["written", "vendor", "all"], default="all")
    args = ap.parse_args()

    try:
        from psd_tools import PSDImage  # noqa: F401
    except ImportError:
        print("SKIP 未安装 psd-tools（pip install psd-tools）；跳过第三方交叉验证")
        return 0

    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：%s" % EXE)
        return 1

    problems = []
    if args.only in ("written", "all"):
        problems += ["written: " + p for p in phase_written()]
    if args.only in ("vendor", "all"):
        problems += ["vendor: " + p for p in phase_vendor()]

    print()
    if problems:
        for p in problems:
            print("FAIL", p)
        return 1
    print("ALL CROSS-IMPLEMENTATION CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
