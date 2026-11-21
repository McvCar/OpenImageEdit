"""分组拖进分组（嵌套）回归。

覆盖用户报的问题：分组也要能拖到别的分组里，并同步更新渲染状态。
  group-move:2,1  —— 组 2 进组 1（parent 改动，成员直属组不变）
  防环            —— 组 1 移进自己的后代组 2 必须被拒绝
  显隐同步        —— 移动后隐藏/显示外层组，整棵子树（含嵌套成员）跟随，
                     显示时按 hidden_snapshot 还原
  撤销 / 重做     —— parent 变化进撤销栈

跑法：python scripts/probe_group_nest.py
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_group_nest.png")

OPS = [
    "layers-dump",          # ck0 只有背景
    "layer-new", "layer-new", "layer-new", "layer-new",
    "multi-select:1,2",
    "layer-group",          # 组 1：层 1/2
    "multi-select:3,4",
    "layer-group",          # 组 2：层 3/4
    "groups-dump",          # ck1 两组都在顶层（p=0）
    "group-move:2,1",       # 组 2 进组 1
    "groups-dump",          # ck2 组 2.p=1
    "group-move:1,2",       # 防环：组 1 进自己的后代 → 拒绝
    "groups-dump",          # ck3 状态不变
    "group-visible:1,0",    # 隐藏组 1（整棵子树）
    "layers-dump",          # ck4 层 1-4 全隐藏
    "group-visible:1,1",    # 显示组 1
    "layers-dump",          # ck5 全恢复显示
    "group-move:2,0",       # 组 2 移回顶层
    "groups-dump",          # ck6 组 2.p=0
    "undo",
    "groups-dump",          # ck7 撤销 → 组 2.p=1
    "redo",
    "groups-dump",          # ck8 重做 → 组 2.p=0
]


def dumps_at(events, n):
    assert len(events) > n, "缺少第 %d 份 groups-dump（共 %d 份）" % (n + 1, len(events))
    return {g["id"]: g for g in events[n]["groups"]}


def dump_at(dumps, n):
    assert len(dumps) > n, "缺少第 %d 份 layers-dump（共 %d 份）" % (n + 1, len(dumps))
    return {x["i"]: x for x in dumps[n]["layers"]}


def main():
    fixture(path=FIX)
    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    proc = subprocess.Popen(
        [EXE, FIX, "--probe-layers", "--script=" + ";".join(OPS)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=90)
    time.sleep(0.3)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    gdumps = parse_events(lines, "groups-dump")
    ldumps = parse_events(lines, "layers-dump")
    assert len(gdumps) == 6, "应有 6 份 groups-dump，实得 %d" % len(gdumps)
    assert len(ldumps) == 3, "应有 3 份 layers-dump，实得 %d" % len(ldumps)

    ck1 = dumps_at(gdumps, 0)
    ck2 = dumps_at(gdumps, 1)
    ck3 = dumps_at(gdumps, 2)
    ck6 = dumps_at(gdumps, 3)
    ck7 = dumps_at(gdumps, 4)
    ck8 = dumps_at(gdumps, 5)

    assert ck1[1]["p"] == 0 and ck1[2]["p"] == 0, "ck1 两组初始都在顶层: %s" % ck1
    assert ck2[2]["p"] == 1, "ck2 组2 应嵌进组1: %s" % ck2[2]
    assert ck3[2]["p"] == 1 and ck3[1]["p"] == 0, \
        "ck3 防环：组1 移进后代组2 必须被拒绝: %s" % ck3
    assert ck6[2]["p"] == 0, "ck6 组2 移回顶层: %s" % ck6[2]
    assert ck7[2]["p"] == 1, "ck7 撤销 → 组2 回到组1: %s" % ck7[2]
    assert ck8[2]["p"] == 0, "ck8 重做 → 组2 回到顶层: %s" % ck8[2]

    # 显隐同步：隐藏外层组 1 → 整棵子树（含嵌套组 2 的成员）全隐藏；
    # 显示时按 hidden_snapshot 全部还原。
    ck4 = dump_at(ldumps, 1)
    ck5 = dump_at(ldumps, 2)
    for i in (1, 2, 3, 4):
        assert ck4[i]["v"] is False, "ck4 层%d 应随外层组隐藏: %s" % (i, ck4[i])
        assert ck5[i]["v"] is True, "ck5 层%d 应随外层组恢复: %s" % (i, ck5[i])

    print("PASS  组嵌套移动 / 防环 / 子树显隐同步 / 撤销重做", flush=True)


if __name__ == "__main__":
    main()
