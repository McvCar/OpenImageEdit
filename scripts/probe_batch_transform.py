"""多选批量变换探针（移动 / 缩放 / 旋转 = 整组一个整体，一条历史）。

覆盖用户定稿的三条要求：
  1. 选中多个图层时，拖动 / 缩放 / 旋转把它们当**一个整体**处理；
  2. 选中分组同理（`select_group` 会把整组成员写进多选集合，走的是同一条
     批量路径，这里用 `multi-select` 直接构造等价的多选集合）；
  3. 批量包围盒的**透明区**也算点到整组 —— 那是 QML 侧 `batchBoxHit` 的
     判据，本探针只验几何；「点透明区不换层」由手测覆盖。

每个手势只允许留下**一条**历史：每次手势后 `undo` 一次，两层必须同时回到
起手位置。若实现退化成「每层一条 op」，这里只会复位一层 → 直接 FAIL。

跑法：
    python scripts/probe_batch_transform.py
输出：
    每个检查点一行摘要 + PASS / FAILED（纯断言，不截图）
"""
import json
import math
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_batch_transform.psd")

# Ctrl（Qt::ControlModifier）= 临时关吸附，位移才精确是 40/30。
MOD_CTRL = 67108864

# PSD fixture：5 层，外接框各不相同（见 make_psd_fixture.small_spec）。
# 文档索引 = 1..5 自下而上（0 号是透明占位背景，locked，不参与批量）；
# PSD 里第 1 段是「负偏移+越界」，所以「普通图层」落在 4、「半透明」落在 3。
A_IX = 4          # 普通图层     (30,20,120,160)
A_NAME = "普通图层"
B_IX = 3          # 半透明 50%   (90,60,100,140)
B_NAME = "半透明 50%"
UNION0 = (30.0, 20.0, 160.0, 180.0)     # 两层并集框
PIVOT = (110.0, 110.0)                  # 并集框中心 = 批量旋转轴心
SCALE_TO = (320.0, 360.0)               # 缩放目标框（2 倍）
ROT_DEG = 30.0

OPS = [
    "layers-dump",                          # ck0 基线
    "multi-select:%d,%d" % (A_IX, B_IX),
    "layer-sel-dump",                       # ck1 多选集合
    "batch-gesture:0,40,30,%d" % MOD_CTRL,  # 批量移动
    "layers-dump",                          # ck2 移动后（绝对值）
    "undo",                                 # **一次**撤销整组复位
    "layers-dump",                          # ck3 撤销后 == 基线
    "multi-select:%d,%d" % (A_IX, B_IX),
    "batch-gesture:1,%g,%g" % SCALE_TO,     # 批量缩放：并集框 → 320x360
    "layers-dump",                          # ck4 缩放后
    "undo",
    "layers-dump",                          # ck5 撤销后 == 基线
    "multi-select:%d,%d" % (A_IX, B_IX),
    "batch-gesture:2,%g,0" % ROT_DEG,       # 批量旋转 30°
    "layers-dump",                          # ck6 旋转后
    "undo",
    "layers-dump",                          # ck7 撤销后 == 基线
]

# layers-dump 的到达顺序（上面 OPS 里 7 次）。
D_BASE, D_MOVE, D_MOVE_UNDO, D_SCALE, D_SCALE_UNDO, D_ROT, D_ROT_UNDO = range(7)


def fixture(path=FIX):
    """用仓库自带的 PSD 生成器造一份 5 层 fixture（免 PIL、免 psd-tools）。"""
    import make_psd_fixture as mk

    mk.write_psd(path, 260, 400, mk.small_spec())


def by_index(rows):
    return {r["i"]: r for r in rows}


def boxes(dump):
    """{i: (x, y, w, h)}，只取这次关心的两层。"""
    m = by_index(dump)
    out = {}
    for i in (A_IX, B_IX):
        r = m.get(i)
        if r is not None:
            out[i] = (float(r["x"]), float(r["y"]), float(r["w"]), float(r["h"]))
    return out


def angles(dump):
    m = by_index(dump)
    return {i: float(m.get(i, {}).get("a", 0.0)) for i in (A_IX, B_IX)}


def near(a, b, tol=1.5):
    return abs(a - b) <= tol


def rot_center(cx, cy, px, py, deg):
    r = math.radians(deg)
    dx, dy = cx - px, cy - py
    return (px + dx * math.cos(r) - dy * math.sin(r),
            py + dx * math.sin(r) + dy * math.cos(r))


def fmt(box):
    if box is None:
        return "None"
    return "(%.1f,%.1f %.1fx%.1f)" % box


def main():
    print("[1] fixture", flush=True)
    fixture()

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")

    print("[2] launch", flush=True)
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-batch", "--script=" + ";".join(OPS)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=120)
    time.sleep(0.4)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    fails = []

    def want(cond, msg):
        if not cond:
            fails.append(msg)

    dumps = parse_events(lines, "layers-dump")
    gests = parse_events(lines, "batch-gesture")
    sels = parse_events(lines, "layer-sel-dump")

    if len(dumps) < 7:
        for line in lines[-25:]:
            print("  |", line)
        print("FAIL 只收到 %d 个 layers-dump（期望 7）" % len(dumps))
        return 1
    if len(gests) < 3:
        for line in lines[-25:]:
            print("  |", line)
        print("FAIL 只收到 %d 个 batch-gesture（期望 3）" % len(gests))
        return 1

    base = boxes(dumps[D_BASE]["layers"])
    rows0 = by_index(dumps[D_BASE]["layers"])
    print("ck0  baseline  a=%s b=%s" % (fmt(base.get(A_IX)), fmt(base.get(B_IX))))
    want(rows0.get(A_IX, {}).get("n", "").startswith(A_NAME),
         "ck0 索引 %d 应是「%s」，实际 %r" % (A_IX, A_NAME, rows0.get(A_IX, {}).get("n")))
    want(rows0.get(B_IX, {}).get("n", "").startswith(B_NAME),
         "ck0 索引 %d 应是「%s」，实际 %r" % (B_IX, B_NAME, rows0.get(B_IX, {}).get("n")))
    want(rows0.get(0, {}).get("l") is True, "ck0 背景层应 locked（不参与批量）")
    if base.get(A_IX) and base.get(B_IX):
        a0, b0 = base[A_IX], base[B_IX]
        ux, uy = min(a0[0], b0[0]), min(a0[1], b0[1])
        ux1 = max(a0[0] + a0[2], b0[0] + b0[2])
        uy1 = max(a0[1] + a0[3], b0[1] + b0[3])
        want(near(ux, UNION0[0]) and near(uy, UNION0[1])
             and near(ux1 - ux, UNION0[2]) and near(uy1 - uy, UNION0[3]),
             "ck0 fixture 并集框应为 %s，实际 (%.1f,%.1f %.1fx%.1f)"
             % (UNION0, ux, uy, ux1 - ux, uy1 - uy))
        pivot = ((ux + ux1) / 2.0, (uy + uy1) / 2.0)
        want(near(pivot[0], PIVOT[0]) and near(pivot[1], PIVOT[1]),
             "ck0 旋转轴心应为 %s，实际 (%.1f,%.1f)" % (PIVOT, pivot[0], pivot[1]))

    if sels:
        print("ck1  selected ids=%s primary=%s count=%s"
              % (sels[0].get("ids"), sels[0].get("primary"), sels[0].get("count")))
        want(sels[0].get("count") == 2, "ck1 应选中 2 层，实际 %s" % sels[0].get("count"))
    else:
        fails.append("ck1 缺失 layer-sel-dump")

    # ---- ck2 批量移动：两层同位移、尺寸不变 ----
    g = gests[0]
    print("ck2  move  union=%s" % (g.get("union"),))
    want(g.get("began") is True, "ck2 批量移动应起手成功（began=false）")
    want(g.get("count") == 2, "ck2 批量目标应 2 层，实际 %s" % g.get("count"))
    before = g.get("before") or []
    after = g.get("after") or []
    want(len(before) == 2 and len(after) == 2,
         "ck2 手势内层数应 2，实际 %d/%d" % (len(before), len(after)))
    for n, (b0, a0) in enumerate(zip(before, after)):
        want(near(a0[0] - b0[0], 40.0) and near(a0[1] - b0[1], 30.0),
             "ck2 第 %d 层位移应 (40,30)，实际 (%.1f,%.1f)"
             % (n, a0[0] - b0[0], a0[1] - b0[1]))
        want(near(a0[2], b0[2]) and near(a0[3], b0[3]),
             "ck2 第 %d 层尺寸不该被移动改变（%s → %s）" % (n, b0[2:], a0[2:]))
    mv = boxes(dumps[D_MOVE]["layers"])
    print("ck2  after   a=%s b=%s" % (fmt(mv.get(A_IX)), fmt(mv.get(B_IX))))
    for i in (A_IX, B_IX):
        if mv.get(i) and base.get(i):
            want(near(mv[i][0] - base[i][0], 40.0) and near(mv[i][1] - base[i][1], 30.0),
                 "ck2 文档里层 %d 的位移应 (40,30)，实际 (%.1f,%.1f)"
                 % (i, mv[i][0] - base[i][0], mv[i][1] - base[i][1]))

    # ---- ck3 一次撤销整组复位 ----
    un = boxes(dumps[D_MOVE_UNDO]["layers"])
    print("ck3  undo    a=%s b=%s" % (fmt(un.get(A_IX)), fmt(un.get(B_IX))))
    for i in (A_IX, B_IX):
        want(un.get(i) is not None and base.get(i) is not None
             and all(near(un[i][k], base[i][k]) for k in range(4)),
             "ck3 **一次**撤销应同时复位两层：层 %d %s → %s"
             % (i, fmt(base.get(i)), fmt(un.get(i))))

    # ---- ck4 批量缩放：并集框 (30,20) 为原点放大 2 倍 ----
    g = gests[1]
    print("ck4  scale union=%s" % (g.get("union"),))
    want(g.get("began") is True, "ck4 批量缩放应起手成功（began=false）")
    gb = g.get("before") or []
    ga = g.get("after") or []
    want(len(ga) == 2, "ck4 缩放后层数应 2，实际 %d" % len(ga))
    for n, (b0, a0) in enumerate(zip(gb, ga)):
        ex = UNION0[0] + (b0[0] - UNION0[0]) * 2.0
        ey = UNION0[1] + (b0[1] - UNION0[1]) * 2.0
        want(near(a0[0], ex) and near(a0[1], ey)
             and near(a0[2], b0[2] * 2.0) and near(a0[3], b0[3] * 2.0),
             "ck4 第 %d 层应为 (%.1f,%.1f %.1fx%.1f)，实际 %s"
             % (n, ex, ey, b0[2] * 2.0, b0[3] * 2.0, fmt(tuple(a0))))
    sc = boxes(dumps[D_SCALE]["layers"])
    print("ck4  after   a=%s b=%s" % (fmt(sc.get(A_IX)), fmt(sc.get(B_IX))))
    if sc.get(A_IX) and sc.get(B_IX):
        a0, b0 = sc[A_IX], sc[B_IX]
        ux, uy = min(a0[0], b0[0]), min(a0[1], b0[1])
        ux1 = max(a0[0] + a0[2], b0[0] + b0[2])
        uy1 = max(a0[1] + a0[3], b0[1] + b0[3])
        want(near(ux, UNION0[0]) and near(uy, UNION0[1])
             and near(ux1 - ux, SCALE_TO[0], 2.0) and near(uy1 - uy, SCALE_TO[1], 2.0),
             "ck4 缩放后并集框应 %s，实际 (%.1f,%.1f %.1fx%.1f)"
             % (SCALE_TO, ux, uy, ux1 - ux, uy1 - uy))

    # ---- ck5 撤销 ----
    un = boxes(dumps[D_SCALE_UNDO]["layers"])
    print("ck5  undo    a=%s b=%s" % (fmt(un.get(A_IX)), fmt(un.get(B_IX))))
    for i in (A_IX, B_IX):
        want(un.get(i) is not None and base.get(i) is not None
             and all(near(un[i][k], base[i][k]) for k in range(4)),
             "ck5 **一次**撤销应同时复位两层：层 %d %s → %s"
             % (i, fmt(base.get(i)), fmt(un.get(i))))

    # ---- ck6 批量旋转：整体转 30°，层心绕并集中心转 ----
    g = gests[2]
    print("ck6  rotate union=%s" % (g.get("union"),))
    ga = g.get("after") or []
    want(len(ga) == 2, "ck6 旋转后层数应 2，实际 %d" % len(ga))
    rt = boxes(dumps[D_ROT]["layers"])
    ang = angles(dumps[D_ROT]["layers"])
    print("ck6  after   a=%s b=%s angles=%s"
          % (fmt(rt.get(A_IX)), fmt(rt.get(B_IX)), ang))
    for i in (A_IX, B_IX):
        b0 = base.get(i)
        a0 = rt.get(i)
        if not b0 or not a0:
            fails.append("ck6 层 %d 缺数据" % i)
            continue
        c0 = (b0[0] + b0[2] / 2.0, b0[1] + b0[3] / 2.0)
        ec = rot_center(c0[0], c0[1], PIVOT[0], PIVOT[1], ROT_DEG)
        ac = (a0[0] + a0[2] / 2.0, a0[1] + a0[3] / 2.0)
        want(near(ac[0], ec[0], 2.0) and near(ac[1], ec[1], 2.0),
             "ck6 层 %d 旋转后中心应 (%.1f,%.1f)，实际 (%.1f,%.1f)"
             % (i, ec[0], ec[1], ac[0], ac[1]))
        want(a0[2] > b0[2] + 1.0 and a0[3] > b0[3] + 1.0,
             "ck6 层 %d 转 %g° 后外接框应变大：%s → %s"
             % (i, ROT_DEG, fmt(b0), fmt(a0)))
        want(near(ang[i], ROT_DEG, 0.2),
             "ck6 层 %d 累计旋转角应 %g°，实际 %.3f" % (i, ROT_DEG, ang[i]))

    # ---- ck7 撤销 ----
    un = boxes(dumps[D_ROT_UNDO]["layers"])
    ang = angles(dumps[D_ROT_UNDO]["layers"])
    print("ck7  undo    a=%s b=%s angles=%s"
          % (fmt(un.get(A_IX)), fmt(un.get(B_IX)), ang))
    for i in (A_IX, B_IX):
        want(un.get(i) is not None and base.get(i) is not None
             and all(near(un[i][k], base[i][k]) for k in range(4)),
             "ck7 **一次**撤销应同时复位两层：层 %d %s → %s"
             % (i, fmt(base.get(i)), fmt(un.get(i))))
        want(near(ang[i], 0.0, 0.05),
             "ck7 撤销后层 %d 的旋转角应回 0，实际 %.3f" % (i, ang[i]))

    done = parse_events(lines, "script-done")
    if done:
        print("script-done:",
              {k: done[-1].get(k) for k in ("layers", "selected_layer", "undo", "redo")})
    else:
        fails.append("没有 script-done，脚本可能中途 panic")

    for f in fails:
        print("FAIL", f)
    print("PASS" if not fails else "FAILED (%d)" % len(fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
