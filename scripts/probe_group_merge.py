"""分组上点「合并选中的图层」应把组就地变成普通图层。

覆盖用户报的 bug：在分组上合并时，当前是在组里新增一个合并层、组还在
（成员数变 1）。期望：组标题行消失，合并结果变成一个组外普通图层。

断言：
  ck1  建组后 groups-dump 有 1 个组、2 个成员；
  ck2  group-select 整组选中后 layer-merge；
  ck3  合并后 groups-dump 无组、layers-group-dump 里结果层 g=0（组外），
       层数减 1，layers-merged 事件 group=true；
  ck4  一次 undo 后组与两个成员层完全复原、层数回到建组后。

跑法：
    python scripts/probe_group_merge.py
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402
from probe_group_select import fixture, FIX, GID, MEMBERS  # noqa: E402

OPS = [
    "layers-dump",                       # 0 基线
    "multi-select:%d,%d" % MEMBERS,
    "layer-group",
    "groups-dump",                       # 1 建组后
    "layers-group-dump",                 # 2 建组后层表
    "group-select:%d" % GID,             # 整组选中
    "layer-sel-dump",                    # 3 选中集合
    "layer-merge",                       # ★合并（自身打印 layers-merged 事件）
    "groups-dump",                       # 4 合并后组表（应为空）
    "layers-group-dump",                 # 5 合并后层表
    "undo",
    "groups-dump",                       # 6 撤销后组表
    "layers-group-dump",                 # 7 撤销后层表
]


def main():
    fixture()
    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-batch", "--script=" + ";".join(OPS)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=120)
    time.sleep(0.3)
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
    lgd = parse_events(lines, "layers-group-dump")
    sels = parse_events(lines, "layer-sel-dump")
    merged = parse_events(lines, "layers-merged")

    print("events: dumps=%d groups=%d lgd=%d sels=%d merged=%d"
          % (len(dumps), len(groups), len(lgd), len(sels), len(merged)))
    if len(groups) < 3 or len(lgd) < 3 or not merged:
        for line in lines[-30:]:
            print("  |", line)
        print("FAIL 事件不全 events: dumps=%d groups=%d lgd=%d sels=%d merged=%d"
              % (len(dumps), len(groups), len(lgd), len(sels), len(merged)))
        return 1

    # ck1 建组
    g1 = groups[0].get("groups") or []
    print("ck1 建组 groups=%d mids=%s"
          % (len(g1), g1[0].get("mids") if g1 else None))
    want(len(g1) == 1, "ck1 应 1 个组")
    want(len(g1[0].get("mids") or []) == len(MEMBERS), "ck1 组成员数")

    # ck2 整组选中
    sel = sels[0].get("count")
    print("ck2 整组选中 count=%s" % sel)
    want(sel == len(MEMBERS), "ck2 选中集合应 == 成员数")

    # ck3 合并后
    print("ck3 layers-merged %s" % merged[0])
    want(merged[0].get("group") is True, "ck3 整组合并应标记 group=true")
    g_after = groups[1].get("groups") or []
    print("ck3 合并后 groups=%d" % len(g_after))
    want(len(g_after) == 0, "ck3 合并后组应消失，实际 %d 个" % len(g_after))
    rows_after = lgd[1].get("layers") or []
    base_rows = lgd[0].get("layers") or []
    print("ck3 层数 %d → %d" % (len(base_rows), len(rows_after)))
    want(len(rows_after) == len(base_rows) - 1, "ck3 层数应减 1")
    # 结果层落在合并事件的 at 下标，检查它的 g 字段 = 0（组外）
    at = merged[0].get("at")
    merged_rows = [r for r in rows_after if r.get("i") == at]
    print("ck3 结果层 (i=%s) %s" % (at, merged_rows[0] if merged_rows else None))
    want(bool(merged_rows) and merged_rows[0].get("g") == 0,
         "ck3 合并结果层应在组外 (g=0)")

    # ck4 撤销复原
    g_undo = groups[2].get("groups") or []
    rows_undo = lgd[2].get("layers") or []
    print("ck4 撤销后 groups=%d 层数=%d" % (len(g_undo), len(rows_undo)))
    want(len(g_undo) == 1, "ck4 撤销后组应复原")
    want(len(rows_undo) == len(base_rows), "ck4 撤销后层数应复原")

    if fails:
        print("FAIL:")
        for f in fails:
            print("  -", f)
        return 1
    print("ALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
