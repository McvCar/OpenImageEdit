"""图层分组探针 —— 重点是「普通编辑模式也能用」。

fixture 只是一张普通 PNG（打开时只有一层背景），用 `layer-new` 随手建
几层再编组。全程不碰 PSD，所以这里通过 = 分组功能不依赖 PSD 分层态。

覆盖：
    编组 / 组成员归属 / 折叠展开 / 整组显隐（含成员各自显隐）/
    重命名 / 点组选中整组 / 撤销重做 / 解散分组 / 解散可撤销

跑法：
    python scripts/probe_layer_groups.py
输出：
    每个检查点一行摘要 + PASS / FAILED
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_layer_groups.png")

# NEXT_GROUP_ID 是进程内计数，探针进程里第一次编组必然拿到 1。
GID = 1

OPS = [
    "layers-dump",                       # ck0 只有背景
    "layer-new", "layer-new", "layer-new", "layer-new",
    "layers-dump",                       # ck1 背景 + 4 层
    "multi-select:1,2",
    "layer-group",
    "groups-dump",                       # gk0 一个组、2 个成员
    "layers-dump",                       # ck2 层 1/2 带同一个 g
    "group-collapse:%d,1" % GID,
    "groups-dump",                       # gk1 已折叠
    "group-collapse:%d,0" % GID,
    "groups-dump",                       # gk2 已展开
    "group-visible:%d,0" % GID,
    "layers-dump",                       # ck3 成员隐藏、其它层仍可见
    "group-visible:%d,1" % GID,
    "layers-dump",                       # ck4 成员恢复可见
    "group-rename:%d:按钮组" % GID,
    "groups-dump",                       # gk3 名字改了
    "group-select:%d" % GID,
    "layer-sel-dump",                    # sk0 选中整组（2 层）
    "undo",                              # 撤销重命名
    "groups-dump",                       # gk4 名字回到「组 1」
    "redo",
    "layer-ungroup",
    "groups-dump",                       # gk5 空
    "layers-dump",                       # ck5 所有层 g=0
    "undo",                              # 撤销解散
    "groups-dump",                       # gk6 组回来了
    "layers-dump",                       # ck6 层 1/2 又带 g
]


def main():
    print("[1] fixture", flush=True)
    fixture(path=FIX)

    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")

    print("[2] launch", flush=True)
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-layers", "--script=" + ";".join(OPS)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=60)
    time.sleep(0.6)

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    layers = parse_events(lines, "layers-dump")
    groups = parse_events(lines, "groups-dump")
    sels = parse_events(lines, "layer-sel-dump")
    fails = []

    def ck(events, n, tag):
        if len(events) <= n:
            fails.append("%s%d 缺失（只收到 %d 个）" % (tag, n, len(events)))
            return None
        return events[n]

    def want(cond, msg):
        if not cond:
            fails.append(msg)

    def layer_rows(n):
        ev = ck(layers, n, "ck")
        return ev["layers"] if ev else None

    def group_rows(n):
        ev = ck(groups, n, "gk")
        return ev["groups"] if ev else []

    r0 = layer_rows(0)
    print("ck0  %s" % (len(r0) if r0 is not None else "—"))
    want(r0 is not None and len(r0) == 1, "ck0 普通图片应只有背景一层")

    r1 = layer_rows(1)
    print("ck1  %s" % (len(r1) if r1 is not None else "—"))
    want(r1 is not None and len(r1) == 5, "ck1 新建 4 层后应有 5 行")

    g0 = group_rows(0)
    print("gk0  %s" % [(g["id"], g["cnt"], g["n"]) for g in g0])
    want(len(g0) == 1, "gk0 应有且只有一个分组")
    want(bool(g0) and g0[0]["cnt"] == 2, "gk0 组里应有 2 个成员")
    want(bool(g0) and len(g0[0]["mids"]) == 2, "gk0 mids 应有 2 个稳定 id")

    r2 = layer_rows(2)
    print("ck2  g=%s" % ([r.get("g") for r in r2] if r2 else "—"))
    if r2:
        tagged = [r for r in r2 if r.get("g", 0) > 0]
        want(len(tagged) == 2, "ck2 应恰好有 2 层带组标记")
        want(len({r["g"] for r in tagged}) == 1, "ck2 两层应属于同一个组")

    g1 = group_rows(1)
    print("gk1  collapsed=%s" % ([g["c"] for g in g1]))
    want(bool(g1) and g1[0]["c"] is True, "gk1 折叠应生效")
    g2 = group_rows(2)
    print("gk2  collapsed=%s" % ([g["c"] for g in g2]))
    want(bool(g2) and g2[0]["c"] is False, "gk2 展开应生效")

    r3 = layer_rows(3)
    print("ck3  v=%s" % ([r["v"] for r in r3] if r3 else "—"))
    if r3:
        hidden = [r for r in r3 if not r["v"]]
        want(len(hidden) == 2, "ck3 整组隐藏应只关掉 2 个成员")
    r4 = layer_rows(4)
    print("ck4  v=%s" % ([r["v"] for r in r4] if r4 else "—"))
    want(r4 is not None and all(r["v"] for r in r4), "ck4 显示整组后应全部可见")

    g3 = group_rows(3)
    print("gk3  n=%s" % ([g["n"] for g in g3]))
    want(bool(g3) and "按钮组" in g3[0]["n"], "gk3 重命名应生效")

    sel = ck(sels, 0, "sk")
    print("sk0  count=%s" % (sel["count"] if sel else "—"))
    want(sel is not None and sel["count"] == 2, "sk0 点组标题应选中整组 2 层")

    g4 = group_rows(4)
    print("gk4  n=%s" % ([g["n"] for g in g4]))
    want(bool(g4) and "按钮组" not in g4[0]["n"], "gk4 撤销重命名应还原组名")
    want(bool(g4) and g4[0]["cnt"] == 2, "gk4 撤销重命名不应把组弄丢")

    g5 = group_rows(5)
    print("gk5  groups=%d" % len(g5))
    want(len(g5) == 0, "gk5 解散后应没有分组")
    r5 = layer_rows(5)
    print("ck5  g=%s" % ([r.get("g") for r in r5] if r5 else "—"))
    want(r5 is not None and all(r.get("g", 0) == 0 for r in r5),
         "ck5 解散后所有层都不应带组标记")

    g6 = group_rows(6)
    print("gk6  groups=%d" % len(g6))
    want(len(g6) == 1, "gk6 撤销解散应把组找回来")
    r6 = layer_rows(6)
    print("ck6  g=%s" % ([r.get("g") for r in r6] if r6 else "—"))
    want(r6 is not None
         and len([r for r in r6 if r.get("g", 0) > 0]) == 2,
         "ck6 撤销解散后成员归属也要回来")

    # 分组命令本身不能报失败。
    ok = parse_events(lines, "script-group")
    want(bool(ok) and all(e["ok"] for e in ok), "编组命令返回了失败")
    un = parse_events(lines, "script-ungroup")
    want(bool(un) and all(e["ok"] for e in un), "解散命令返回了失败")

    print()
    if fails:
        print("FAILED (%d):" % len(fails))
        for f in fails:
            print("  -", f)
        return 1
    print("PASS  分组功能在普通编辑模式全部检查点通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
