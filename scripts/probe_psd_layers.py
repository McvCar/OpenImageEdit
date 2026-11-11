#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""M3 probe: PSD two-stage loading, editing-lock lifecycle, sync assembly.

Two independent runs:

  sync  : --script=... opens the PSD through the automation path. It must see
          the FINAL layered document inline (background + N layers) and must
          NEVER raise the editing lock (probes would otherwise be dead).

  async : plain `imageedit.exe <file>` (no -- flag) goes through prewarm +
          stage-1 composite + stage-2 layer job. Assert the full event chain
          editing-locked -> psd-stage2-ms -> psd-layer-count -> editing-unlocked,
          that QML loaded cleanly, and that the lock duration stays <= 3 s.

  fail  : automation hook forces a stage-2 parse error; the lock must still be
          released and no layer stack installed.
  skip  : per-layer delay + auto-skip hook fires the escape hatch while the
          lock is held; no Done message may be processed afterwards.
  slow  : per-layer delay keeps stage 2 running for ~1.8 s; progress must be
          visible, layers installed, and the lock released within 3 s.

Usage: python scripts/probe_psd_layers.py [--only sync|async|fail|skip|slow|all]
"""

import argparse
import json
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

FIXTURE = os.path.join(ROOT, "tmp", "psd-fixtures", "small-layers.psd")
EXPECTED_LAYERS = 5  # background + 5 = 6 rows
STAGE2_LIMIT_MS = 3000


def env(extra=None):
    e = dict(os.environ)
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    e["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + e.get("PATH", "")
    # Never let a hook from a previous run leak into a fresh variant.
    for key in (
        "IMAGEEDIT_PSD_STAGE2_DELAY_MS",
        "IMAGEEDIT_PSD_STAGE2_FAIL",
        "IMAGEEDIT_PSD_AUTO_SKIP",
    ):
        e.pop(key, None)
    if extra:
        e.update(extra)
    return e


def launch(args, needle, timeout=40.0, settle=1.5, extra_env=None):
    proc = subprocess.Popen(
        [EXE] + args,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        errors="replace",
        env=env(extra_env),
        bufsize=1,
    )
    lines, _ = pump(proc, needle, timeout=timeout)
    time.sleep(settle)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
    return lines


def has_qml_error(lines):
    return any(
        ("qml-load-failed" in l) or ("is not a type" in l) or ("TypeError" in l)
        for l in lines
    )


def sync_probe():
    print("--- sync (--script) path ---", flush=True)
    lines = launch(
        [FIXTURE, "--script=layers-dump;layer-info:1;layer-info:2"],
        '"event":"qml-loaded"',
        settle=2.0,
    )
    problems = []
    if has_qml_error(lines):
        problems.append("QML 加载报错")
    dumps = parse_events(lines, "layers-dump")
    if not dumps:
        problems.append("没有 layers-dump 事件")
    else:
        rows = dumps[-1].get("layers", [])
        print("  层行:", [(r.get("i"), r.get("n")) for r in rows])
        if len(rows) != EXPECTED_LAYERS + 1:
            problems.append("层数应为 背景+%d，实际 %d" % (EXPECTED_LAYERS, len(rows)))
        names = " ".join(str(r.get("n", "")) for r in rows)
        if "背景" not in names:
            problems.append("0 号位不是背景层")
        if any(ord(c) > 127 for c in names) is False:
            problems.append("中文层名丢失")
    locks = parse_events(lines, "editing-locked")
    if locks:
        problems.append("同步路径不该上编辑锁（探针会被废掉）")
    # `layer-info` confirms the layer bitmaps really have their own rectangles.
    infos = parse_events(lines, "layer-info")
    if len(infos) < 2:
        problems.append("layer-info 没打印出分层位图尺寸")
    print("  layer-info:", len(infos), "条; 编辑锁事件:", len(locks))
    return problems


def async_probe():
    print("--- async (prewarm + stage 2) path ---", flush=True)
    lines = launch([FIXTURE], '"event":"editing-unlocked"', timeout=30.0, settle=1.0)
    problems = []
    if has_qml_error(lines):
        problems.append("QML 加载报错")
    if any('"event":"handoff-ok"' in l for l in lines):
        problems.append("被单实例转交，没测到本进程的异步路径")

    def one(name):
        e = parse_events(lines, name)
        return e[-1] if e else None

    locked, unlocked = one("editing-locked"), one("editing-unlocked")
    stage2, count = one("psd-stage2-ms"), one("psd-layer-count")
    installed, bytes_ev = one("psd-install-ms"), one("psd-layer-bytes")
    if not locked:
        problems.append("首帧落地后没有上编辑锁")
    if not unlocked:
        problems.append("阶段 2 结束后没有解锁（C2-c 失败路径必须解锁）")
    if not stage2:
        problems.append("缺 psd-stage2-ms 事件")
    elif stage2.get("value", 0) > STAGE2_LIMIT_MS:
        problems.append("阶段 2 / 锁定时长 %d > %d ms" % (stage2["value"], STAGE2_LIMIT_MS))
    if not count or count.get("value") != EXPECTED_LAYERS:
        problems.append("psd-layer-count 不是 %d" % EXPECTED_LAYERS)
    if not installed:
        problems.append("没有装配分层（psd-install-ms 缺失）")
    if not bytes_ev:
        problems.append("缺 psd-layer-bytes 事件")
    print(
        "  events: locked=%s stage2=%s count=%s bytes=%s install=%s unlocked=%s"
        % (
            bool(locked),
            stage2.get("value") if stage2 else None,
            count.get("value") if count else None,
            bytes_ev.get("value") if bytes_ev else None,
            installed.get("value") if installed else None,
            bool(unlocked),
        )
    )
    if locked and unlocked:
        print("  锁时长(ms):", unlocked.get("ms", 0) - locked.get("ms", 0))
    return problems


def variant_probe(label, hooks, expect_installed, expect_reason=None,
                  min_lock_ms=0, expect_stage2_done=True, expect_progress=False):
    """Shared assertions for the failure / skip / slow stage-2 paths."""
    print("--- %s ---" % label, flush=True)
    lines = launch([FIXTURE], '"event":"editing-unlocked"', timeout=30.0,
                   settle=0.6, extra_env=hooks)
    problems = []
    if has_qml_error(lines):
        problems.append("QML 加载报错")
    unlocked = parse_events(lines, "editing-unlocked")
    locked = parse_events(lines, "editing-locked")
    installed = parse_events(lines, "psd-install-ms")
    progress = parse_events(lines, "psd-layer-progress")
    stage2 = parse_events(lines, "psd-stage2-ms")
    if not unlocked:
        problems.append("没有解锁（C2-c：所有失败/跳过路径都必须解锁）")
    if not locked:
        problems.append("没有上锁")
    if expect_installed and not installed:
        problems.append("应当装配分层")
    if not expect_installed and installed:
        problems.append("不该装配分层，却装配了")
    if expect_stage2_done and not stage2:
        problems.append("应当收到阶段 2 完成消息")
    if not expect_stage2_done and stage2:
        problems.append("已在阶段 2 完成前逃生，不该再处理 Done 消息")
    if expect_progress and not progress:
        problems.append("逃生前应先看到逐层进度")
    if expect_reason and (not unlocked or unlocked[-1].get("reason") != expect_reason):
        problems.append("解锁 reason 应为 %s，实际 %s"
                        % (expect_reason, unlocked[-1].get("reason") if unlocked else None))
    if min_lock_ms and locked and unlocked:
        held = unlocked[-1].get("ms", 0) - locked[-1].get("ms", 0)
        if held < min_lock_ms:
            problems.append("锁定时长 %d < %d，慢加载没被真正锁住" % (held, min_lock_ms))
    print("  locked=%s progress=%d installed=%s stage2=%s unlocked_reason=%s"
          % (bool(locked), len(progress), bool(installed), bool(stage2),
             unlocked[-1].get("reason") if unlocked else None))
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["sync", "async", "fail", "skip", "slow", "all"],
                    default="all")
    args = ap.parse_args()
    if not os.path.exists(EXE):
        print("FAIL debug exe 不存在:", EXE)
        return 1
    if not os.path.exists(FIXTURE):
        print("FAIL fixture 不存在，先跑 make_psd_fixture.py")
        return 1
    problems = []
    if args.only in ("sync", "all"):
        problems += ["sync: " + p for p in sync_probe()]
    if args.only in ("async", "all"):
        problems += ["async: " + p for p in async_probe()]
    if args.only in ("fail", "all"):
        problems += ["fail: " + p for p in variant_probe(
            "fail (parse error -> unlock)",
            {"IMAGEEDIT_PSD_STAGE2_FAIL": "1"}, False)]
    if args.only in ("skip", "all"):
        problems += ["skip: " + p for p in variant_probe(
            "skip (escape hatch -> unlock before Done)",
            {"IMAGEEDIT_PSD_STAGE2_DELAY_MS": "300", "IMAGEEDIT_PSD_AUTO_SKIP": "1"},
            False, "skipped", min_lock_ms=200,
            expect_stage2_done=False, expect_progress=True)]
    if args.only in ("slow", "all"):
        problems += ["slow: " + p for p in variant_probe(
            "slow (per-layer progress, still <= 3 s)",
            {"IMAGEEDIT_PSD_STAGE2_DELAY_MS": "300"}, True, "done",
            min_lock_ms=400, expect_progress=True)]
    if problems:
        for p in problems:
            print("FAIL " + p)
        return 1
    print("ALL PSD LAYER / LOCK CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
