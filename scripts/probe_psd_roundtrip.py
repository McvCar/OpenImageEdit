#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""End-to-end PSD layered write-back probe (M5 / M6).

Verifies the whole B-plan promise: open a layered PSD, edit it, Ctrl+S, and
still find the layers when the file is reopened — in OUR reader AND in a raw
byte-level PSD parse that does not use any Rust code.

Phases (each launches a fresh process, so "reopen" is a real reopen):

  write      : --script opens the fixture, toggles one layer's visibility,
               saves over the source. Waits for the `saved` event.
  raw        : independent big-endian PSD record parser checks the written
               file: magic, layer count, Chinese names via 8BIMluni, flags
               (Adobe: bit 1 SET = hidden), no PNG-bytes-in-.psd bug.
  reopen     : --script reopens the saved file and dumps the layer rows.
  stable     : save again and reopen once more; the layer count must NOT grow
               (the synthetic transparent background must not be written).
  composite  : a stage-2 failure leaves a composite-only PSD; --probe-save
               must be REJECTED and the original bytes must be untouched.

  skip       : user escape hatch -> composite-only -> write-back rejected,
               but a plain PNG export is still possible (covered by the
               `validate_save_target` unit tests in paths.rs).

Usage: python scripts/probe_psd_roundtrip.py [--only write|reopen|stable|composite|saveas|lossy|all]
"""

import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

FIXTURE = os.path.join(ROOT, "tmp", "psd-fixtures", "small-layers.psd")
WORK_DIR = os.path.join(ROOT, "tmp", "psd-roundtrip")
WORK = os.path.join(WORK_DIR, "work.psd")
COMPOSITE = os.path.join(WORK_DIR, "composite.psd")
EXPECTED_LAYERS = 5  # PSD layer records (the synthetic background is skipped)
TOGGLED_NAME = "普通图层"
HOOKS = (
    "IMAGEEDIT_PSD_STAGE2_DELAY_MS",
    "IMAGEEDIT_PSD_STAGE2_FAIL",
    "IMAGEEDIT_PSD_AUTO_SKIP",
    "IMAGEEDIT_PSD_SYNC_COMPOSITE",
)


# --------------------------------------------------------------------------
# Independent PSD record parser (no Rust, no vendored crate).
# --------------------------------------------------------------------------

def u16(b, o):
    return struct.unpack_from(">H", b, o)[0]


def i16(b, o):
    return struct.unpack_from(">h", b, o)[0]


def u32(b, o):
    return struct.unpack_from(">I", b, o)[0]


def parse_psd(path):
    with open(path, "rb") as fp:
        b = fp.read()
    assert b[:4] == b"8BPS", "not a PSD (PNG bytes written into .psd?)"
    # Header: 12 channels, 14 height, 18 width, 22 depth, 24 color mode.
    width, height, depth, mode = u32(b, 18), u32(b, 14), u16(b, 22), u16(b, 24)
    o = 26
    o += 4 + u32(b, o)  # color mode data
    o += 4 + u32(b, o)  # image resources
    section_len = u32(b, o)
    o += 4
    section_end = o + section_len
    info_len = u32(b, o)
    o += 4
    count = i16(b, o)
    o += 2
    layers = []
    for _ in range(abs(count)):
        top, left, bottom, right = struct.unpack_from(">iiii", b, o)
        o += 16
        nch = u16(b, o)
        o += 2
        channels = []
        for _ in range(nch):
            channels.append((i16(b, o), u32(b, o + 2)))
            o += 6
        blend = b[o + 4:o + 8].decode("latin1")
        o += 8
        opacity, clipping, flags = b[o], b[o + 1], b[o + 2]
        o += 4
        extra_len = u32(b, o)
        o += 4
        extra_end = o + extra_len
        o += 4 + u32(b, o)                 # layer mask data
        o += 4 + u32(b, o)                 # blending ranges
        nlen = b[o]
        o += 1
        name = b[o:o + nlen].decode("latin1")
        o += nlen
        o += (4 - ((nlen + 1) % 4)) % 4    # Pascal name padding
        while o + 12 <= extra_end:
            sig = b[o:o + 4]
            if sig not in (b"8BIM", b"8B64"):
                break
            key = b[o + 4:o + 8]
            ln = u32(b, o + 8)
            data = b[o + 12:o + 12 + ln]
            if key == b"luni":
                units = u32(data, 0)
                name = data[4:4 + 2 * units].decode("utf-16-be")
            o += 12 + ln
        o = extra_end
        layers.append({
            "name": name,
            "rect": (top, left, bottom, right),
            "opacity": opacity,
            "flags": flags,
            "visible": (flags & 2) == 0,   # Adobe: bit 1 SET = HIDDEN
            "blend": blend,
            "channels": channels,
        })
    return {
        "width": width, "height": height, "depth": depth, "mode": mode,
        "layers": layers, "info_len": info_len, "section_end": section_end,
        "size": len(b),
    }


def sha(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


# --------------------------------------------------------------------------
# Process helpers
# --------------------------------------------------------------------------

def env(extra=None):
    e = dict(os.environ)
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    e["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + e.get("PATH", "")
    for key in HOOKS:
        e.pop(key, None)
    if extra:
        e.update(extra)
    return e


def launch(args, needle, timeout=40.0, settle=0.8, extra_env=None):
    proc = subprocess.Popen(
        [EXE] + args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, errors="replace", env=env(extra_env), bufsize=1,
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


def fatal(lines):
    return any(
        ("qml-load-failed" in l) or ("is not a type" in l) or ("TypeError" in l)
        for l in lines
    )


def run_script(path, script, needle, extra_env=None):
    return launch([path, "--script=" + script], needle, extra_env=extra_env)


def marks(lines):
    """`bridge.mark(name)` 走的是 event=mark + name 的形式。"""
    return [e.get("name") for e in parse_events(lines, "mark")]


def rows(lines):
    dumps = parse_events(lines, "layers-dump")
    return dumps[-1].get("layers", []) if dumps else []


# --------------------------------------------------------------------------
# Phases
# --------------------------------------------------------------------------

def phase_write():
    print("--- write: edit + Ctrl+S ---", flush=True)
    os.makedirs(WORK_DIR, exist_ok=True)
    shutil.copyfile(FIXTURE, WORK)
    before = sha(WORK)
    lines = run_script(
        WORK,
        "layers-dump;select-layer:2;toggle-visible-selected;save",
        '"event":"saved"',
    )
    problems = []
    if fatal(lines):
        problems.append("QML 加载报错")
    saved = parse_events(lines, "saved")
    if not saved:
        problems.append("没有 saved 事件（后台编码 / 原子写没完成）")
    if not parse_events(lines, "psd-encode-ms"):
        problems.append("缺 psd-encode-ms 埋点")
    if parse_events(lines, "save-error"):
        problems.append("分层 PSD 写回被错误拒绝")
    if sha(WORK) == before:
        problems.append("文件没有被改写")
    print("  saved=%s encode=%s" % (
        bool(saved),
        [e.get("value") for e in parse_events(lines, "psd-encode-ms")],
    ))
    return problems


def phase_raw():
    print("--- raw: independent PSD byte-level parse ---", flush=True)
    problems = []
    info = parse_psd(WORK)
    names = [l["name"] for l in info["layers"]]
    if info["depth"] != 8 or info["mode"] != 3:
        problems.append("不是 8bit RGB PSD")
    if len(info["layers"]) != EXPECTED_LAYERS:
        problems.append("PSD 层记录 %d 个，应为 %d（透明占位背景不该写出）"
                        % (len(info["layers"]), EXPECTED_LAYERS))
    if any(ord(c) > 127 for c in "".join(names)) is False:
        problems.append("中文层名（8BIMluni）丢失")
    toggled = next((l for l in info["layers"] if l["name"] == TOGGLED_NAME), None)
    if toggled is None:
        problems.append("找不到 %s" % TOGGLED_NAME)
    elif toggled["visible"]:
        problems.append("编辑后的隐藏状态没写回（flags=%d）" % toggled["flags"])
    # Every declared channel length must include the 2-byte compression marker.
    for l in info["layers"]:
        w = l["rect"][3] - l["rect"][1]
        h = l["rect"][2] - l["rect"][0]
        for cid, clen in l["channels"]:
            if clen != 2 + w * h:
                problems.append("%s 通道长度 %d != 2+%d*%d"
                                % (l["name"], clen, w, h))
                break
    print("  canvas=%dx%d layers=%d names=%s" % (
        info["width"], info["height"], len(info["layers"]), names))
    return problems


def phase_reopen():
    print("--- reopen: layers must survive ---", flush=True)
    lines = run_script(WORK, "layers-dump;layer-info:2", '"event":"layers-dump"')
    problems = []
    if fatal(lines):
        problems.append("QML 加载报错")
    got = rows(lines)
    # 1 synthetic background + N real layers.
    if len(got) != EXPECTED_LAYERS + 1:
        problems.append("重开后图层行 %d，应为 %d" % (len(got), EXPECTED_LAYERS + 1))
    names = [r.get("n") for r in got]
    if any(ord(c) > 127 for c in "".join(str(n) for n in names)) is False:
        problems.append("重开后中文层名丢失")
    toggled = next((r for r in got if r.get("n") == TOGGLED_NAME), None)
    if toggled is None:
        problems.append("重开后找不到 %s" % TOGGLED_NAME)
    elif toggled.get("v"):
        problems.append("重开后 %s 仍可见（隐藏状态没保留）" % TOGGLED_NAME)
    print("  rows=%d names=%s" % (len(got), [r.get("n") for r in got]))
    return problems


def phase_stable():
    print("--- stable: second save/reopen must not add layers ---", flush=True)
    problems = []
    lines = run_script(WORK, "save", '"event":"saved"')
    if not parse_events(lines, "saved"):
        problems.append("第二次保存失败")
    lines = run_script(WORK, "layers-dump", '"event":"layers-dump"')
    got = rows(lines)
    if len(got) != EXPECTED_LAYERS + 1:
        problems.append("第二代重开后图层行 %d（应恒为 %d）"
                        % (len(got), EXPECTED_LAYERS + 1))
    info = parse_psd(WORK)
    if len(info["layers"]) != EXPECTED_LAYERS:
        problems.append("第二代 PSD 记录 %d 层，应为 %d"
                        % (len(info["layers"]), EXPECTED_LAYERS))
    print("  rows=%d records=%d" % (len(got), len(info["layers"])))
    return problems


def phase_composite():
    print("--- composite-only: write-back must be rejected ---", flush=True)
    shutil.copyfile(FIXTURE, COMPOSITE)
    before = sha(COMPOSITE)
    # Keep the composite placeholder on the synchronous automation path
    # (psd_layered=false, exactly what a failed / skipped stage 2 leaves).
    # The Rust gate must refuse the .psd overwrite and leave the bytes intact.
    lines = run_script(
        COMPOSITE, "save", '"event":"save-error"',
        extra_env={"IMAGEEDIT_PSD_SYNC_COMPOSITE": "1"},
    )
    problems = []
    if fatal(lines):
        problems.append("QML 加载报错")
    errors = parse_events(lines, "save-error")
    if not errors:
        problems.append("合并预览 PSD 竟然允许写回（会破坏原图层）")
    elif "合并预览" not in errors[-1].get("detail", ""):
        problems.append("拒绝提示不是合并预览专用文案：%s"
                        % errors[-1].get("detail"))
    if parse_events(lines, "saved"):
        problems.append("合并预览 PSD 被写出了 saved 事件")
    if sha(COMPOSITE) != before:
        problems.append("被拒绝的保存改写了原文件")
    if parse_events(lines, "psd-install-ms"):
        problems.append("不该装配分层，却装配了")
    # The synchronous automation path must never raise the editing lock.
    if parse_events(lines, "editing-locked"):
        problems.append("同步路径不该上编辑锁")
    print("  errors=%d file_unchanged=%s locked=%s" % (
        len(errors), sha(COMPOSITE) == before, bool(parse_events(lines, "editing-locked"))))
    return problems


def phase_saveas():
    print("--- save-as: plain image -> fresh layered PSD ---", flush=True)
    from PIL import Image
    os.makedirs(WORK_DIR, exist_ok=True)
    src = os.path.join(WORK_DIR, "plain.png")
    dst = os.path.join(WORK_DIR, "plain-saved.psd")
    if os.path.exists(dst):
        os.remove(dst)
    img = Image.new("RGBA", (32, 24))
    img.putdata([
        (x * 7 % 256, y * 11 % 256, (x + y) * 5 % 256, 255)
        for y in range(24) for x in range(32)
    ])
    img.save(src)
    lines = run_script(src, "save-as:" + dst.replace("\\", "/"),
                       '"event":"saved"')
    problems = []
    if fatal(lines):
        problems.append("QML 加载报错")
    if not parse_events(lines, "saved"):
        problems.append("另存为 PSD 没有产生 saved 事件")
    if parse_events(lines, "save-error"):
        problems.append("普通图片另存为 PSD 被错误拒绝")
    if not os.path.exists(dst):
        problems.append("目标 PSD 没有生成")
        return problems
    info = parse_psd(dst)
    if info["width"] != 32 or info["height"] != 24:
        problems.append("另存 PSD 画布尺寸不对：%dx%d"
                        % (info["width"], info["height"]))
    if len(info["layers"]) != 1:
        problems.append("普通图片另存 PSD 应写出 1 层，实际 %d"
                        % len(info["layers"]))
    if info["layers"] and info["layers"][0]["flags"] & 2:
        problems.append("另存的图层不该被写成隐藏")
    print("  psd=%dx%d layers=%d size=%d bytes" % (
        info["width"], info["height"], len(info["layers"]), info["size"]))
    return problems


def phase_lossy():
    """Ctrl+S on a PSD whose load degraded structure must ask first.

    `small-layers.psd` reports degradation notes (clipping mask / group /
    blend modes), so `psdWriteLossy` is true: the QML guard must stop the
    overwrite and leave the bytes alone. With the probe-only accept hook the
    same flow must go through and rewrite the file. A PSD that loaded with no
    degradation at all must NOT be interrupted (otherwise the guard breaks
    every ordinary save).
    """
    print("--- lossy: overwrite guard asks before dropping structure ---", flush=True)
    os.makedirs(WORK_DIR, exist_ok=True)
    work = os.path.join(WORK_DIR, "lossy.psd")
    problems = []

    # 1) 降级 PSD：警告拦住，原文件不动。
    shutil.copyfile(FIXTURE, work)
    before = sha(work)
    guard = launch([work, "--probe-save"], '"event":"psd-install-ms"',
                   timeout=45.0, settle=2.5)
    if fatal(guard):
        problems.append("QML 加载报错")
    if "psd-lossy-save-warn" not in marks(guard):
        problems.append("降级 PSD 按 Ctrl+S 没有弹覆盖警告")
    if parse_events(guard, "saved") or parse_events(guard, "psd-encode-ms"):
        problems.append("警告没拦住，文件被直接覆盖了")
    if sha(work) != before:
        problems.append("被拦住的保存仍然改写了原文件")
    print("  guard warn=%s saved=%s unchanged=%s" % (
        "psd-lossy-save-warn" in marks(guard),
        bool(parse_events(guard, "saved")), sha(work) == before))

    # 2) 确认「仍然覆盖」：写回发生，且确认状态被记住。
    accepted = launch([work, "--probe-save", "--probe-accept-lossy"],
                      '"event":"saved"', timeout=45.0, settle=1.0)
    if fatal(accepted):
        problems.append("QML 加载报错（accept 轮）")
    if not parse_events(accepted, "psd-lossy-save-accepted"):
        problems.append("「仍然覆盖」没有记录确认状态")
    if not parse_events(accepted, "saved"):
        problems.append("确认之后仍然没有写回")
    if sha(work) == before:
        problems.append("确认之后文件没有变化")
    print("  accepted=%s saved=%s rewritten=%s" % (
        bool(parse_events(accepted, "psd-lossy-save-accepted")),
        bool(parse_events(accepted, "saved")), sha(work) != before))

    # 3) 干净 PSD（我们自己写出来的单层文件）：不得被打扰。
    plain = os.path.join(WORK_DIR, "clean.psd")
    from PIL import Image
    png = os.path.join(WORK_DIR, "clean.png")
    Image.new("RGBA", (24, 16), (30, 60, 90, 255)).save(png)
    run_script(png, "save-as:" + plain.replace("\\", "/"), '"event":"saved"')
    plain_before = sha(plain)
    # 先改一点东西再按 Ctrl+S：内容没变时编码结果字节完全一致，sha 对比
    # 分辨不出「写没写」，这里用显隐切换保证字节真的会变。
    clean = launch([plain, "--script=select-layer:1;toggle-visible-selected",
                    "--probe-save"], '"event":"saved"', timeout=45.0, settle=1.0)
    if "psd-lossy-save-warn" in marks(clean):
        problems.append("没有降级的 PSD 也被覆盖警告拦住了")
    if not parse_events(clean, "saved"):
        problems.append("干净的 PSD 按 Ctrl+S 没有直接保存")
    if sha(plain) == plain_before:
        problems.append("干净的 PSD 按 Ctrl+S 没有真正改写")
    print("  clean warn=%s saved=%s rewritten=%s" % (
        "psd-lossy-save-warn" in marks(clean),
        bool(parse_events(clean, "saved")), sha(plain) != plain_before))
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["write", "raw", "reopen", "stable",
                                       "composite", "saveas", "lossy", "all"],
                    default="all")
    args = ap.parse_args()
    if not os.path.exists(EXE):
        print("FAIL debug exe 不存在:", EXE)
        return 1
    if not os.path.exists(FIXTURE):
        print("FAIL fixture 不存在，先跑 make_psd_fixture.py --only small")
        return 1
    problems = []
    if args.only in ("write", "all"):
        problems += ["write: " + p for p in phase_write()]
    if args.only in ("raw", "all"):
        problems += ["raw: " + p for p in phase_raw()]
    if args.only in ("reopen", "all"):
        problems += ["reopen: " + p for p in phase_reopen()]
    if args.only in ("stable", "all"):
        problems += ["stable: " + p for p in phase_stable()]
    if args.only in ("composite", "all"):
        problems += ["composite: " + p for p in phase_composite()]
    if args.only in ("saveas", "all"):
        problems += ["saveas: " + p for p in phase_saveas()]
    if args.only in ("lossy", "all"):
        problems += ["lossy: " + p for p in phase_lossy()]
    if problems:
        for p in problems:
            print("FAIL " + p)
        return 1
    print("ALL PSD ROUND-TRIP CHECKS PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
