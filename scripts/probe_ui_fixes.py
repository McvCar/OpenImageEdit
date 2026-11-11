#!/usr/bin/env python
"""三条用户报的 UI 修复的回归探针（无需真鼠标，走 --probe-* 钩子）。

1. Ctrl+A 全选：有选中图层时先取消图层选中（移动工具开着的前提下）；
   移动工具关掉时保持原样。--- `--probe-select-all` / `--probe-select-all-off`
2. 污点 / 油漆工具「关掉后选区工具没回来」：开一次再关一次，断言 selTool
   回到进组之前用的那个（而不是停在 none）。--- `--probe-toggle-heal`

用法：
    python scripts/probe_ui_fixes.py
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
FIXTURE = os.path.join(ROOT, "probe_ui_fixes.png")


def fixture():
    from PIL import Image, ImageDraw
    im = Image.new("RGB", (800, 600), (238, 238, 238))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 799, 599], outline=(220, 40, 40), width=3)
    for x in range(40, 800, 40):
        d.line([(x, 3), (x, 596)], fill=(226, 226, 226))
    im.save(FIXTURE)


def run(extra, timeout=12.0):
    env = dict(os.environ)
    env["PATH"] = QT_BIN + ";" + env.get("PATH", "")
    cmd = [EXE, FIXTURE, "--probe-keys"] + extra
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, errors="replace",
                           timeout=timeout, env=env, cwd=ROOT)
        out = (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired as e:
        out = ((e.stdout or "") + (e.stderr or ""))
        if isinstance(out, bytes):
            out = out.decode("utf-8", "replace")
    return out


def grab(out, tag):
    """取出 `tag: k=v k=v ...` 那一行，解析成 dict。"""
    for ln in out.splitlines():
        if tag + ":" not in ln:
            continue
        body = ln.split(tag + ":", 1)[1]
        # 行尾还带着 JSON 的 `","ms":…}`，值里不能吃引号。
        return dict(re.findall(r"(\w+)=([^ \"]+)", body))
    return None


def main():
    if not os.path.exists(EXE):
        print("FAIL 找不到 exe：", EXE)
        return 1
    fixture()
    fails = []

    def check(name, ok, detail=""):
        print("%-34s %s %s" % (name, "PASS" if ok else "FAIL", detail))
        if not ok:
            fails.append(name)

    script = "--script=extract:120,100,240,180"

    # ---- 1. Ctrl+A（移动工具开）----
    out = run([script, "--probe-select-all"])
    before = grab(out, "selall-before")
    after = grab(out, "selall-after")
    # A25 起 `layers[0]` 恒为背景，抠图结果落在索引 1，所以这里只要求
    # 「有图层被选中」，不再硬编码 0（那是统一模型之前的单层文档）。
    check("全选前有选中图层",
          bool(before) and before.get("selLayer") not in (None, "-1"), str(before))
    check("全选后取消图层选中",
          bool(after) and after.get("selLayer") == "-1", str(after))
    check("全选后选区存在", bool(after) and after.get("selExists") == "true", str(after))

    # ---- 1b. Ctrl+A（移动工具关）----
    out = run([script, "--probe-select-all-off"])
    after = grab(out, "selall-after")
    check("移动工具关时保留图层选中",
          bool(after) and after.get("selLayer") not in (None, "-1")
          and after.get("moveMode") == "false",
          str(after))

    # ---- 2. 污点工具开→关，选区工具回来 ----
    out = run(["--probe-toggle-heal"])
    opened = grab(out, "paint-toggle-open")
    closed = grab(out, "paint-toggle-close")
    check("污点工具打开", bool(opened) and opened.get("paintTool") == "spot", str(opened))
    check("打开时选区工具让位",
          bool(opened) and opened.get("selTool") == "none", str(opened))
    check("关闭后工具退出",
          bool(closed) and closed.get("paintTool") == "none", str(closed))
    check("关闭后选区工具恢复",
          bool(closed) and closed.get("selTool") in ("rect", "ellipse", "lasso",
                                                     "polygon", "wand", "quick"),
          str(closed))

    # ---- 3. 点空白：移动工具关掉时保留图层选中 ----
    script3 = "--script=extract:120,100,240,180"
    out = run([script3, "--probe-empty-click"])
    b = grab(out, "emptyclick-before")
    a = grab(out, "emptyclick-after")
    check("移动工具开：点空白前有图层+选区",
          bool(b) and b.get("selLayer") not in (None, "-1")
          and b.get("selExists") == "true", str(b))
    check("移动工具开：点空白取消图层选中",
          bool(a) and a.get("selLayer") == "-1", str(a))

    out = run([script3, "--probe-empty-click-off"])
    b = grab(out, "emptyclick-before")
    a = grab(out, "emptyclick-after")
    check("移动工具关：随手点是区不选图层",
          bool(b) and b.get("pick") == "null", str(b))
    check("移动工具关：点空白保留图层选中",
          bool(a) and a.get("selLayer") not in (None, "-1")
          and a.get("selLayer") == b.get("selLayer")
          and a.get("moveMode") == "false",
          str(a))
    check("移动工具关：选区照旧被清掉",
          bool(a) and a.get("selExists") == "false", str(a))

    print()
    if fails:
        print("RESULT FAIL:", ", ".join(fails))
        return 1
    print("RESULT ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
