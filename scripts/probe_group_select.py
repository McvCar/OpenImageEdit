"""分组选中 = 一个整体（包围盒 + 批量变换）探针。

覆盖用户报的两个问题：
  2. 图层面板里**单选**时不该同时选中「组 + 组里的图层」；
  3. 选中分组应该出现**包围盒选区**，并能在此基础上拖动 / 缩放 / 旋转整组。

做法：`select_group` 是画布侧的唯一真相 —— 它把「整组非影子成员」写进多选
集合，QML 的并集框（`findLayerBox`）与批量手势（`batch_targets_of`）都读这个
集合。所以这里断言：

  ck1  `layer-group` 建组后 `groups-dump` 有成员表；
  ck2  `group-select:<gid>` 后多选集合 == 组成员数，主选中层 == 组内最上层；
  ck3  多选集合里**没有**合成线（`synthetic` 组样式影子层）—— 影子层往往
       是整幅画布，混进来会让组的框撑满整张图；
  ck4  组选中后 `batch-gesture` 整组移动：两层同位移、尺寸不变；
  ck5  一次 `undo` 两层同时复位（组 = 一条历史）。
另跑一次窗口截图（`probe_group_select.png`）用于肉眼确认：组行高亮、成员行
不亮、画布上是一圈包住整组的框。

跑法：
    python scripts/probe_group_select.py
"""
import json
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_group_select.psd")
SHOT = os.path.join(ROOT, "probe_group_select.png")
MOD_CTRL = 67108864

# fixture：背景(0) + 三个普通层(1..3)，自下而上 1→3。
# 用 1、2 两层建组，3 号留在组外（对照「组外图层不受影响」）。
GID = 1                 # 空文档里第一个组
MEMBERS = (1, 2)
OUTSIDE = 3

OPS = [
    "layers-dump",                       # ck0 基线
    "multi-select:%d,%d" % MEMBERS,
    "layer-group",
    "groups-dump",                       # ck1 组元数据
    "select-layer:-1",                   # 先清空选择（下面验证 group-select 会重建）
    "layer-sel-dump",                    # ck2a 期望空
    "group-select:%d" % GID,
    "layer-sel-dump",                    # ck2b 期望 == 成员数
    "batch-gesture:0,25,15,%d" % MOD_CTRL,
    "layers-dump",                       # ck4 移动后
    "undo",
    "layers-dump",                       # ck5 撤销后 == 基线
    # ck6：Ctrl 逐个点满成员 —— 面板应亮**组行**（group-focus = gid），
    # 而不是「两行图层都亮、组行不亮」（用户报的困惑点）。
    "select-layer:-1",
    "select-mode:%d,0" % MEMBERS[0],
    "select-mode:%d,1" % MEMBERS[1],
    "layer-sel-dump",
    # ck7：Ctrl+点组行 = 把整组**并入**当前选择（分组也能批量选中）。
    "select-layer:%d" % OUTSIDE,
    "group-select:%d,add" % GID,
    "layer-sel-dump",
    # ck8：新建图层落在当前选中层的位置（选中第 1 层 → 新层插到 1 号位，
    # 不再永远跑到整摞最上面）。
    "select-layer:1",
    "new-layer",
    "layers-group-dump",
    # ck9：复制组内单层 → 粘贴，不该把上级分组一起带出来（只多一层，
    # 不新建组）。
    "select-layer:1",
    "copy",
    "paste",
    "groups-dump",
    "layers-group-dump",
    # ck10：选中整组复制 → 粘贴，应新建一个**同名**组并把成员收进去。
    "select-layer:-1",
    "group-select:%d" % GID,
    "copy",
    "paste",
    "groups-dump",
    "layers-group-dump",
    "layer-sel-dump",
]

D_BASE, D_MOVE, D_UNDO = 0, 1, 2


def fixture(path=FIX):
    """三层 PSD（900x600）：几何刻意分开，并集框才看得出「组 = 一圈框」。"""
    import make_psd_fixture as mk

    geo = [
        ("绿色条", 470, 300, 300, 70),
        ("橙色方块", 180, 520, 220, 160),
        ("蓝色圆", 190, 150, 220, 220),
    ]
    layers = [dict(name=n, top=t, left=l, w=w, h=h) for (n, t, l, w, h) in geo]
    mk.write_psd(path, 900, 600, layers)


def by_index(rows):
    return {r["i"]: r for r in rows}


def box_of(row):
    return (float(row["x"]), float(row["y"]), float(row["w"]), float(row["h"]))


def near(a, b, tol=1.5):
    return abs(a - b) <= tol


def dump_chunk(lines, event, index):
    hits = parse_events(lines, event)
    return hits[index] if len(hits) > index else None


def main():
    print("[1] fixture", flush=True)
    fixture()

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")

    print("[2] launch (assertions)", flush=True)
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
    groups = parse_events(lines, "groups-dump")
    sels = parse_events(lines, "layer-sel-dump")
    gests = parse_events(lines, "batch-gesture")
    created = parse_events(lines, "group-create")

    if len(dumps) < 3 or not groups or len(sels) < 2 or not gests:
        for line in lines[-30:]:
            print("  |", line)
        print("FAIL 事件不全：dumps=%d groups=%d sels=%d gests=%d"
              % (len(dumps), len(groups), len(sels), len(gests)))
        return 1

    # ---- ck1 建组 ----
    gid = (created[0].get("id") if created else None)
    print("ck1  group-create id=%s members=%s"
          % (gid, created[0].get("members") if created else None))
    want(gid == GID, "ck1 首个组 id 应为 %d，实际 %s" % (GID, gid))
    glist = groups[0].get("groups") or []
    want(len(glist) == 1, "ck1 应只有 1 个组，实际 %d" % len(glist))
    if glist:
        g = glist[0]
        mids = g.get("mids") or []
        print("ck1  group id=%s name=%s mids=%s" % (g.get("id"), g.get("n"), mids))
        want(g.get("id") == GID, "ck1 groups-dump 里组 id 应为 %d" % GID)
        want(len(mids) == len(MEMBERS),
             "ck1 组成员应 %d 个，实际 %d" % (len(MEMBERS), len(mids)))

    # ---- ck2a 清空后多选为空 ----
    n0 = sels[0].get("count")
    print("ck2a cleared count=%s primary=%s" % (n0, sels[0].get("primary")))
    want(n0 in (0, 1) and sels[0].get("primary") in (-1, None),
         "ck2a select-layer:-1 后不该还有选中层，实际 count=%s primary=%s"
         % (n0, sels[0].get("primary")))

    # ---- ck2b group-select 把整组写进多选 ----
    s = sels[1]
    ids = s.get("ids") or []
    print("ck2b selected ids=%s primary=%s count=%s"
          % (ids, s.get("primary"), s.get("count")))
    want(s.get("count") == len(MEMBERS),
         "ck2b 组选中应 %d 个图层，实际 %s" % (len(MEMBERS), s.get("count")))
    want(len(ids) == len(MEMBERS), "ck2b ids 应 %d 个，实际 %d"
         % (len(MEMBERS), len(ids)))

    rows = by_index(dumps[D_BASE]["layers"])
    top_member = None
    for r in dumps[D_BASE]["layers"]:      # 显示序 = 顶 → 底
        if r["id"] in ids and not r.get("syn"):
            top_member = r["i"]
            break
    print("ck2b 组内最上层索引=%s primary=%s" % (top_member, s.get("primary")))
    want(top_member is not None and s.get("primary") == top_member,
         "ck2b 主选中层应为组内最上层 %s，实际 %s"
         % (top_member, s.get("primary")))

    # ---- ck3 多选集合里不能有影子层 / 背景 ----
    bad = [r["i"] for r in dumps[D_BASE]["layers"]
           if r["id"] in ids and (r.get("syn") or r.get("base"))]
    print("ck3  synthetic/base in selection:", bad)
    want(not bad, "ck3 组选中集合里混进了影子层/背景：%s" % bad)
    want(0 not in [r["i"] for r in dumps[D_BASE]["layers"] if r["id"] in ids],
         "ck3 背景层不该被组选中")

    # ---- ck4 整组移动 ----
    g = gests[0]
    before = g.get("before") or []
    after = g.get("after") or []
    print("ck4  move union=%s count=%s" % (g.get("union"), g.get("count")))
    want(g.get("began") is True, "ck4 组选中后批量移动应起手成功")
    want(g.get("count") == len(MEMBERS),
         "ck4 批量目标应 %d 层，实际 %s" % (len(MEMBERS), g.get("count")))
    want(len(before) == len(MEMBERS) and len(after) == len(MEMBERS),
         "ck4 手势内层数应 %d，实际 %d/%d"
         % (len(MEMBERS), len(before), len(after)))
    for n, (b0, a0) in enumerate(zip(before, after)):
        want(near(a0[0] - b0[0], 25.0) and near(a0[1] - b0[1], 15.0),
             "ck4 第 %d 层位移应 (25,15)，实际 (%.1f,%.1f)"
             % (n, a0[0] - b0[0], a0[1] - b0[1]))
        want(near(a0[2], b0[2]) and near(a0[3], b0[3]),
             "ck4 第 %d 层尺寸不该被移动改变（%s → %s）" % (n, b0[2:], a0[2:]))

    base = {r["i"]: r for r in dumps[D_BASE]["layers"]}
    moved = {r["i"]: r for r in dumps[D_MOVE]["layers"]}
    for i in MEMBERS:
        want(near(box_of(moved[i])[0] - box_of(base[i])[0], 25.0)
             and near(box_of(moved[i])[1] - box_of(base[i])[1], 15.0),
             "ck4 文档里成员层 %d 应位移 (25,15)" % i)
    if OUTSIDE in base and OUTSIDE in moved:
        want(all(near(box_of(moved[OUTSIDE])[k], box_of(base[OUTSIDE])[k])
                 for k in range(4)),
             "ck4 组外图层 %d 不该跟着动：%s → %s"
             % (OUTSIDE, box_of(base[OUTSIDE]), box_of(moved[OUTSIDE])))

    # ---- ck5 一次撤销整组复位 ----
    un = {r["i"]: r for r in dumps[D_UNDO]["layers"]}
    for i in MEMBERS:
        want(all(near(box_of(un[i])[k], box_of(base[i])[k]) for k in range(4)),
             "ck5 **一次**撤销应同时复位成员层 %d：%s → %s"
             % (i, box_of(base[i]), box_of(un[i])))

    # ---- ck6 Ctrl 逐个点满一组 → 面板亮组行（group-focus = 组 id） ----
    focus = parse_events(lines, "group-focus")
    focus_ids = [f.get("id") for f in focus]
    s6 = sels[2]
    print("ck6  ctrl 点满两层后 count=%s focus=%s（全部 focus=%s）"
          % (s6.get("count"), focus_ids[-1] if focus_ids else None, focus_ids))
    want(s6.get("count") == len(MEMBERS),
         "ck6 Ctrl 点满组成员后应选中 %d 层，实际 %s"
         % (len(MEMBERS), s6.get("count")))
    want(GID in focus_ids,
         "ck6 整组成员全选中时应报告 group-focus=%d（组行高亮），实际 %s"
         % (GID, focus_ids))

    # ---- ck7 Ctrl+点组行 = 整组并入当前选择（分组批量选中） ----
    s7 = sels[3]
    print("ck7  组外层 + Ctrl 点组后 count=%s primary=%s"
          % (s7.get("count"), s7.get("primary")))
    want(s7.get("count") == len(MEMBERS) + 1,
         "ck7 「一层 + 一整组」应为 %d 层，实际 %s"
         % (len(MEMBERS) + 1, s7.get("count")))
    want(focus_ids.count(GID) >= 2,
         "ck7 Ctrl 点组行后仍应报告 group-focus=%d，实际 %s" % (GID, focus_ids))

    # ---- ck8 新建图层落在选中层的位置（不再恒顶） ----
    new_ev = parse_events(lines, "layer-new")
    lg = parse_events(lines, "layers-group-dump")
    if not new_ev or len(lg) < 1:
        fails.append("ck8 没拿到 layer-new / layers-group-dump")
    else:
        at = new_ev[0].get("index")
        print("ck8  new-layer index=%s（选中第 1 层）" % at)
        want(at == 1, "ck8 新层应插在选中层的位置（1 号位），实际 %s" % at)
        rows = lg[0].get("layers") or []
        want(len(rows) == 5, "ck8 加层后应有 5 层，实际 %d" % len(rows))
        want([r["i"] for r in rows].count(at) == 1,
             "ck8 新层应真的落在 %s 号位" % at)

    # ---- ck9 复制组内单层 → 粘贴不带上级分组、不新建组 ----
    g9 = parse_events(lines, "groups-dump")
    pasted = parse_events(lines, "pasted")
    if len(g9) < 2 or len(lg) < 2 or not pasted:
        fails.append("ck9 没拿到 paste 后的 groups/layers dump")
    else:
        print("ck9  粘贴后组数=%d（复制单层不该新建组），pasted=%s"
              % (len(g9[1].get("groups") or []), pasted[0].get("index")))
        want(len(g9[1].get("groups") or []) == 1,
             "ck9 复制**单层**粘贴后组数应还是 1，实际 %d"
             % len(g9[1].get("groups") or []))
        rows = lg[1].get("layers") or []
        want(len(rows) == 6, "ck9 粘贴后应有 6 层，实际 %d" % len(rows))
        pi = pasted[0].get("index")
        prow = next((r for r in rows if r.get("i") == pi), None)
        print("ck9  粘贴层 i=%s 所属组=%s（应为 0：只复制当前图层）"
              % (pi, prow.get("g") if prow else "?"))
        want(prow is not None and prow.get("g") == 0,
             "ck9 单层粘贴不该挂进原来的组，实际 g=%s"
             % (prow.get("g") if prow else "?"))

    # ---- ck10 复制整组 → 粘贴新建同名组 ----
    if len(g9) < 3 or len(lg) < 3:
        fails.append("ck10 没拿到整组粘贴后的 dump")
    else:
        gl = g9[2].get("groups") or []
        prev = {g.get("id"): g for g in (g9[1].get("groups") or [])}
        src_mids = (prev.get(GID) or {}).get("mids") or []
        s10 = sels[4]
        print("ck10 整组粘贴后组数=%d，选择数=%s（源组 %d 层）"
              % (len(gl), s10.get("count"), len(src_mids)))
        want(len(gl) == 2, "ck10 复制整组粘贴应新建 1 个组（共 2 个），实际 %d" % len(gl))
        if len(gl) == 2:
            names = [g.get("n") for g in gl]
            want(names[0] == names[1],
                 "ck10 新组应沿用名字 %s，实际 %s" % (names[0], names[1]))
            newg = gl[1]
            want(len(newg.get("mids") or []) == len(src_mids),
                 "ck10 新组应含 %d 个成员，实际 %d"
                 % (len(src_mids), len(newg.get("mids") or [])))
            want(s10.get("count") == len(src_mids),
                 "ck10 整组粘贴后应选中整组（%d 层），实际 %s"
                 % (len(src_mids), s10.get("count")))
            gid10 = newg.get("id")
            rows = lg[2].get("layers") or []
            attached = [r for r in rows if r.get("g") == gid10]
            want(len(attached) == len(src_mids),
                 "ck10 新组应有 %d 层挂上，实际 %d 层"
                 % (len(src_mids), len(attached)))

    # ---- 截图：组选中态（组行高亮 / 成员行不亮 / 并集框）----
    print("[3] launch (screenshot)", flush=True)
    shot_ops = "multi-select:%d,%d;layer-group;group-select:%d" % (
        MEMBERS[0], MEMBERS[1], GID)
    sp = subprocess.Popen(
        # `--probe-layers` 把图层面板自动打开：截图要能看见「只有组标题行
        # 高亮、成员行不亮」。
        [EXE, FIX, "--probe-batch", "--probe-layers", "--script=" + shot_ops],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    pump(sp, '"event":"script-done"', timeout=90)
    time.sleep(1.2)
    try:
        subprocess.run(
            [sys.executable, os.path.join(ROOT, "scripts", "shot_window.py"),
             str(sp.pid), SHOT],
            timeout=90, check=False,
        )
        print("shot ->", SHOT)
    except Exception as exc:                     # 截图失败不算回归失败
        print("shot skipped:", exc)
    sp.terminate()
    try:
        sp.wait(timeout=5)
    except subprocess.TimeoutExpired:
        sp.kill()

    done = parse_events(lines, "script-done")
    if not done:
        fails.append("没有 script-done，脚本可能中途 panic")

    for f in fails:
        print("FAIL", f)
    print("PASS" if not fails else "FAILED (%d)" % len(fails))
    return 0 if not fails else 1


if __name__ == "__main__":
    sys.exit(main())
