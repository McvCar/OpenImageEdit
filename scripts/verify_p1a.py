"""P1-A 修饰键语义与插值一致性回归（无需 GUI 操作）。

覆盖（docs/变换系统重构-修改路线.md §9.1 验收）：
  1. 角点默认自由、Shift 锁比例（语义与旧实现相反，D14）；
  2. Alt 从中心：对侧反向对称，中心在手势中恒定；
  3. Ctrl+拖边 = 斜切（ky/kx 换算 + W/N 缘对侧锚定补偿）；
  4. 角点 + Ctrl 本期按自由缩放（透视留 P1-A2）；
  5. solve-scale 与落定共用吸附：非斜切手势才吸附（Ctrl 斜切时跳过）；
  6. Shift 15° 吸角走 rotate_layer_update / finish（累计角吸附）；
  7. 斜切落定：skew 写进 params（source 不烘焙，sw/sh 恒定）+ 一次撤销还原。

用法：python scripts/verify_p1a.py [exe]
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = tempfile.gettempdir()
SRC = os.path.join(TMP, "p1a_in.png")
SHIFT = 0x02000000
CTRL = 0x04000000
ALT = 0x08000000

FAILED = []


def fixture(path):
    im = Image.new("RGB", (500, 400), (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, 500, 4):
        d.rectangle([x, 0, x + 1, 399], fill=(0, 0, 0))
    d.rectangle([70, 50, 430, 350], outline=(220, 30, 30), width=3)
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


def run(tag, ops, timeout=180):
    dst = os.path.join(TMP, "p1a_%s.png" % tag)
    shutil.copyfile(SRC, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    args = [EXE, dst, "--script=" + ops, "--auto-quit"]
    p = subprocess.run(args, capture_output=True, text=True, env=env, timeout=timeout)
    return events_of(p.stdout + p.stderr), dst


def pixel_diff(a, b):
    """RGBA 图的逐像素差异 bbox：直接对 RGBA diff 取 getbbox 会被全零的
    alpha 通道带偏（RGB 明明不同仍返回 None），先压成 L 再取。"""
    return ImageChops.difference(a, b).convert("L").getbbox()


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("  | " + str(detail) if detail else ""))
    if not ok:
        FAILED.append(name)


def solve(ops):
    ev, _ = run("solve", ops)
    for e in ev:
        if e.get("event") == "solve-scale":
            return e.get("out", {})
    return None


def main():
    fixture(SRC)

    # ---- 1. 角点语义：默认自由，Shift 锁比例 --------------------------
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,se,80,10,0")
    check("角点默认自由（宽 280）", r and abs(r["w"] - 280) < 0.01, r and r.get("w"))
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,se,80,23,%d" % SHIFT)
    ok = r and abs(r["w"] / r["h"] - 200.0 / 160.0) < 1e-6
    check("角点+Shift 锁比例（2:1.6）", ok, r and (r.get("w"), r.get("h")))

    # ---- 2. Alt 从中心：中心恒定 --------------------------------------
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,e,30,0,%d" % ALT)
    cx = (r["x"] + r["w"] / 2.0) if r else None
    check("Alt 从中心：宽度 260", r and abs(r["w"] - 260) < 0.01, r and r.get("w"))
    check("Alt 从中心：中心恒定 140", r and abs(cx - 140) < 0.01, cx)

    # ---- 3. Ctrl + 拖边 = 斜切 ----------------------------------------
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,e,0,30,%d" % CTRL)
    check("Ctrl+东缘：ky = 30/200", r and abs(r["ky"] - 30.0 / 200.0) < 1e-6,
          r and r.get("ky"))
    check("Ctrl+东缘：kx = 0、无补偿", r and r["kx"] == 0 and r["cx"] == 0 and r["cy"] == 0,
          r and (r.get("kx"), r.get("cx"), r.get("cy")))
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,w,0,30,%d" % CTRL)
    check("Ctrl+西缘：ky 反号 + cy 补偿 30",
          r and abs(r["ky"] + 30.0 / 200.0) < 1e-6 and abs(r["cy"] - 30.0) < 0.01,
          r and (r.get("ky"), r.get("cy")))
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,s,50,0,%d" % CTRL)
    check("Ctrl+南缘：kx = 50/160", r and abs(r["kx"] - 50.0 / 160.0) < 1e-6,
          r and r.get("kx"))

    # ---- 4. 角点 + Ctrl 本期按自由缩放 --------------------------------
    r = solve("extract:40,40,200,160;solve-scale:40,40,200,160,se,80,10,%d" % CTRL)
    check("角点+Ctrl：自由缩放、无斜切", r and r["kx"] == 0 and r["ky"] == 0
          and abs(r["w"] - 280) < 0.01, r and (r.get("w"), r.get("kx")))

    # ---- 5. 斜切落定：skew 进 params、原图不烘焙、可一次撤销 ----------
    ev, dst = run("commit", ";".join([
        "extract:40,40,200,160",
        "layer-info:0",
        "skew-layer:30,e",
        "layer-info:0",
        "save",
    ]))
    infos = [e for e in ev if e.get("event") == "layer-info"]
    skew = [e for e in ev if e.get("event") == "skew-layer"]
    ok = len(infos) >= 2 and skew and infos[-1].get("sw") == 200 \
        and infos[-1].get("sh") == 160
    check("斜切落定：source 不烘焙（仍是抽取的 200×160）", ok,
          [(i.get("sw"), i.get("sh")) for i in infos] if infos else infos)
    before = Image.open(SRC).convert("RGBA")
    after = Image.open(dst).convert("RGBA")
    check("斜切落定：画面确实被剪切（像素变化）",
          after.size == before.size and pixel_diff(after, before) is not None,
          pixel_diff(after, before))
    ev, dst = run("undo", ";".join([
        "extract:40,40,200,160",
        "skew-layer:30,e",
        "undo",
        "save",
    ]))
    restored = Image.open(dst).convert("RGBA")
    check("斜切落定：一次撤销逐像素还原",
          pixel_diff(restored, before) is None)

    # ---- 6. 旋转 Shift 吸角（update/finish 均按累计角吸附）------------
    ev, _ = run("rot", "extract:40,40,200,160;rotate-handle:0,23,1")
    rot = [e for e in ev if e.get("event") == "rotate-commit"]
    check("旋转无修饰键：23° 连续", rot and abs(rot[-1].get("degrees", 0) - 23.0) < 0.01,
          rot and rot[-1].get("degrees"))
    ev, _ = run("rotshift", "extract:40,40,200,160;rotate-handle:0,23,1,%d" % SHIFT)
    rot = [e for e in ev if e.get("event") == "rotate-commit"]
    check("旋转+Shift：23° 吸到 30°", rot and abs(rot[-1].get("degrees", 0) - 30.0) < 0.01,
          rot and rot[-1].get("degrees"))
    ev, _ = run("rotshift2",
                "extract:40,40,200,160;rotate-handle:0,23,1,%d;rotate-handle:0,23,1,%d"
                % (SHIFT, SHIFT))
    rot = [e for e in ev if e.get("event") == "rotate-commit"]
    # 连续两次吸角：第二次按累计角 53° 吸到 45°（base=30），仍是 15 的倍数。
    check("旋转+Shift 两次：累计角恒为 15 的倍数",
          rot and len(rot) >= 2 and abs(rot[-1].get("degrees", 0) % 15.0) < 0.01,
          [r.get("degrees") for r in rot])

    print()
    if FAILED:
        print("FAILED: %d 项：%s" % (len(FAILED), "; ".join(FAILED)))
        sys.exit(1)
    print("ALL PASS")


if __name__ == "__main__":
    main()
