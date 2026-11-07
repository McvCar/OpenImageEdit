"""拖动缩放图层时「实时帧」的层级 / 位置核对（无 GUI 操作）。

复现用户报的两个现象：
  1. 拖动过程中图层画到了背景下面（被背景盖住 → 放大后的内容看不见）；
  2. 松手后画面没有变大，要再移动一次图层才正确。

做法：把一块**纯蓝**区域抽成图层（底下全是黑白条纹背景）。放大到 400×320
后，目标框内必须整片是蓝的——若实时帧里图层没画出来（被背景盖住），框内
会是条纹；若画在错位置，蓝块会出现在别处。

  `scale-hold:w,h`  只 begin+update（不落定），把拖动中那一帧留在屏幕上，
                    日志里给出该帧 BMP 的 URL → 直接断言拖动帧。
  `scale-layer:w,h` begin+update+end，`save` 落盘 → 断言落定后的文件。

用法：python scripts/probe_scale_live.py [exe]
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
SRC = os.path.join(TMP, "scalelive_in.png")

CANVAS = (500, 400)
BOX = (150, 120, 350, 280)        # 被抽取成图层的蓝块（图像坐标 x1,y1,x2,y2）
TARGET = (400, 320)               # 拖动放大后的尺寸（中心不变）
BLUE = (30, 60, 220)

FAILED = []


def fixture(path):
    im = Image.new("RGB", CANVAS, (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, CANVAS[0], 4):
        d.rectangle([x, 0, x + 1, CANVAS[1] - 1], fill=(0, 0, 0))
    d.rectangle([BOX[0], BOX[1], BOX[2] - 1, BOX[3] - 1], fill=BLUE)
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
    dst = os.path.join(TMP, "scalelive_%s.png" % tag)
    shutil.copyfile(SRC, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    args = [EXE, dst, "--script=" + ops, "--auto-quit"]
    p = subprocess.run(args, capture_output=True, text=True, env=env, timeout=timeout)
    return events_of(p.stdout + p.stderr), dst


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + ("  | " + str(detail) if detail else ""))
    if not ok:
        FAILED.append(name)


def is_blue(px, tol=26):
    return (abs(px[0] - BLUE[0]) <= tol and abs(px[1] - BLUE[1]) <= tol
            and abs(px[2] - BLUE[2]) <= tol)


def blue_ratio(img, box):
    """框内「蓝色」像素占比（抽样，避开边界的插值过渡）。"""
    x0, y0, x1, y1 = [int(v) for v in box]
    x0, y0 = max(0, x0), max(0, y0)
    x1, y1 = min(img.width, x1), min(img.height, y1)
    total = hit = 0
    for y in range(y0 + 3, y1 - 3, 3):
        for x in range(x0 + 3, x1 - 3, 3):
            total += 1
            if is_blue(img.getpixel((x, y))):
                hit += 1
    return hit / float(total or 1), total


def center_box():
    cx = (BOX[0] + BOX[2]) / 2.0
    cy = (BOX[1] + BOX[3]) / 2.0
    tw, th = TARGET
    bx = int(round(cx - tw / 2.0))
    by = int(round(cy - th / 2.0))
    return bx, by, bx + tw, by + th


def main():
    fixture(SRC)
    bx, by, x1, y1 = center_box()

    extract = "extract:%d,%d,%d,%d" % (BOX[0], BOX[1], BOX[2] - BOX[0], BOX[3] - BOX[1])

    # ---- 1. 拖动中的实时帧：图层必须画在背景之上，并且是新尺寸 ----------
    ev, _ = run("hold", extract + ";scale-hold:%d,%d" % TARGET)
    holds = [e for e in ev if e.get("event") == "scale-hold"]
    check("拖动帧探针跑通（scale-hold 有日志）", bool(holds), holds[-1] if holds else None)
    if holds:
        url = holds[-1].get("url", "")
        path = url[len("file:///"):] if url.startswith("file:///") else url
        check("拖动帧 BMP 存在", os.path.isfile(path), path)
        if os.path.isfile(path):
            img = Image.open(path).convert("RGB")
            r, n = blue_ratio(img, (bx, by, x1, y1))
            check("拖动中：放大框内是图层内容（不是背景）", n and r > 0.9,
                  "blue=%.2f n=%d" % (r, n))
            # 框外仍是条纹背景（图层没有跑到别处 / 没有铺满整张画布）
            out, n2 = blue_ratio(img, (0, 0, bx - 10, CANVAS[1]))
            check("拖动中：框外仍是背景", n2 and out < 0.1, "outside blue=%.2f" % out)

    # ---- 2. 落定后的文件：内容同样必须变大 --------------------------------
    ev, dst = run("commit", extract + ";scale-layer:%d,%d;save" % TARGET)
    img = Image.open(dst).convert("RGB")
    r, n = blue_ratio(img, (bx + 2, by + 2, x1 - 2, y1 - 2))
    check("落定后：文件里图层已是新尺寸", n and r > 0.9, "blue=%.2f n=%d" % (r, n))

    print()
    if FAILED:
        print("FAILED: %d 项：%s" % (len(FAILED), "; ".join(FAILED)))
        sys.exit(1)
    print("ALL PASS")


if __name__ == "__main__":
    main()
