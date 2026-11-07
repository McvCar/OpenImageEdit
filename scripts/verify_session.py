"""P0-2b1 统一变换会话回归。

三种手势（移动 / 八向缩放 / 旋转）现在共用 `Session::transform` 这一个
互斥会话。本脚本用 `--script=...;transform-state` 断言：

  * 空闲、每种手势落定后：`active=false`（无残留临时合成帧）；
  * `rotate-hold` 故意不 finish 时：`active=true / kind=rotate / special=true`；
  * `transform-abort` 能一次 take 清空会话；
  * 旋转未收尾就直接拖图层，不会把会话留到下一次手势。

用法：python scripts/verify_session.py [exe]
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
SRC = os.path.join(TMP, "session_probe_in.png")


def fixture(path):
    im = Image.new("RGB", (400, 300), (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, 400, 4):
        d.rectangle([x, 0, x + 1, 299], fill=(0, 0, 0))
    d.rectangle([60, 60, 340, 240], outline=(220, 30, 30), width=3)
    im.save(path)


def run(tag, ops):
    dst = os.path.join(TMP, "session_probe_%s.png" % tag)
    shutil.copyfile(SRC, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    script = "extract:20,20,200,160;" + ops + ";save"
    out = subprocess.run(
        [EXE, dst, "--script=" + script, "--auto-quit"],
        capture_output=True, text=True, env=env, timeout=180,
    )
    events = []
    for line in (out.stdout + out.stderr).splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        if ev.get("event") in ("transform-state", "transform-abort", "live-stale"):
            events.append(ev)
    return events


def states(events):
    return [e["state"] for e in events if e.get("event") == "transform-state"]


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (("  " + detail) if detail else ""))
    if not ok:
        raise SystemExit(1)


def main():
    fixture(SRC)

    ev = run("idle", "transform-state")
    s = states(ev)
    check("空闲时无会话", s == [{"active": False}], repr(s))

    ev = run("move", "move-layer:7,3;transform-state")
    s = states(ev)
    check("移动落定后无残留", s == [{"active": False}], repr(s))

    ev = run("scale", "scale-layer:150,100;transform-state")
    s = states(ev)
    check("缩放落定后无残留", s == [{"active": False}], repr(s))

    ev = run("finish", "rotate-handle:0,25;transform-state")
    s = states(ev)
    check("旋转落定后无残留", s == [{"active": False}], repr(s))

    ev = run("hold", "rotate-hold:0,25;transform-state;transform-abort;transform-state")
    s = states(ev)
    check("rotate-hold 期间仅一个 rotate 会话",
          len(s) == 2 and s[0].get("active") is True
          and s[0].get("kind") == "rotate" and s[0].get("special") is True
          and s[0].get("cap", 0) > 0 and s[1] == {"active": False}, repr(s))
    check("transform-abort 命中会话",
          any(e.get("event") == "transform-abort" and e.get("had") is True for e in ev))

    ev = run("stale", "rotate-hold:0,25;move-layer:7,3;transform-state")
    s = states(ev)
    check("未收尾旋转后直接拖图层，会话被接管且清空",
          s == [{"active": False}], repr(s))

    print("PASS: P0-2b1 统一变换会话回归")


if __name__ == "__main__":
    main()
