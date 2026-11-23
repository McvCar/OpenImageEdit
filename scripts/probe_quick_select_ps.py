"""回归：快速选择工具对齐 PS 的三项 —— 取样全部图层、自动增强、笔迹中途 Alt 减选。

`--script=` 依次喂 qsel-opt / qsel-modes，读 quick-select / quick-select-modes
事件的 area 做断言。fixture：白底 + 一层红方块，红块外对「当前图层」是透明，
对「全部图层」是白底 —— 两条取样路径结果差别明显。
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = os.path.join(ROOT, "target", "probe-qsel-ps")
FIXTURE = os.path.join(TMP, "fixture.png")


def make_fixture():
    from PIL import Image, ImageDraw
    os.makedirs(TMP, exist_ok=True)
    im = Image.new("RGB", (400, 300), (250, 250, 250))
    ImageDraw.Draw(im).rectangle([100, 100, 200, 200], fill=(200, 30, 30))
    im.save(FIXTURE)


def run(ops):
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    out = subprocess.run(
        [EXE, FIXTURE, "--script=" + ops, "--auto-quit"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=120,
    )
    text = out.stdout.decode("utf-8", "replace")
    events = []
    for line in text.splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        events.append(ev)
    return events


def area_of(events, event_name):
    return [e["area"] for e in events if e.get("event") == event_name]


def main():
    make_fixture()
    ok = True

    # 1) 取样全部图层：点在红块外。当前图层 = 透明 → 只有笔刷圆盘；
    #    全部图层 = 白底 → 泛洪铺满背景。
    ops = ("extract:100,100,100,100;"
           "qsel-opt:24,40,160,0,0;qsel-modes:50,50,0|8,40,200;"
           "sel-none;qsel-opt:24,40,160,1,0;qsel-modes:50,50,0|8,40,200")
    areas = area_of(run(ops), "quick-select-modes")
    good = len(areas) >= 2 and areas[1] > areas[0] * 5
    print("%-4s sample-all: layer=%s composite=%s" % (
        "PASS" if good else "FAIL", areas[:1], areas[1:2]))
    ok = ok and good

    # 2) 笔迹中途 Alt：同一条笔迹先加后减，面积必须比纯加选小。
    ops = ("extract:100,100,100,100;qsel-opt:24,20,20,0,0;"
           "qsel-modes:150,150,0 170,150,0|10,20,2;"
           "sel-none;"
           "qsel-modes:150,150,0 170,150,0 200,150,2|10,20,2")
    areas = area_of(run(ops), "quick-select-modes")
    good = len(areas) >= 2 and 0 < areas[1] < areas[0]
    print("%-4s mid-stroke Alt: add=%s add-then-sub=%s" % (
        "PASS" if good else "FAIL", areas[:1], areas[1:2]))
    ok = ok and good

    # 3) 自动增强：开关两档都能出合法选区，且不把小选区抹没。
    ops = ("extract:100,100,100,100;"
           "qsel-opt:24,20,20,0,0;qsel-modes:140,140,0 180,180,0|12,20,2;"
           "sel-none;"
           "qsel-opt:24,20,20,0,1;qsel-modes:140,140,0 180,180,0|12,20,2")
    areas = area_of(run(ops), "quick-select-modes")
    good = len(areas) >= 2 and areas[0] > 0 and areas[1] > 0
    print("%-4s auto-enhance: off=%s on=%s" % (
        "PASS" if good else "FAIL", areas[:1], areas[1:2]))
    ok = ok and good

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
