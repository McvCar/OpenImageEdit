"""图层眼睛显示态 / 按行重命名 探针（无头，走 --script 桥路径）。

覆盖本轮三项修复里可用脚本验证的两项：
  问题2 —— 组隐藏时成员 `v`(合成真值)=false 但 `ev`(眼睛图标)=true；
           隐藏期间点眼睛改的是「自身意愿」，显示组后按意愿还原。
  问题3 —— `rename-at:<index>:<name>` 按行下标改名，不依赖当前选中态。

问题1（拖入组高亮）是纯 QML 视觉反馈，靠代码审查 + 手测截图确认，
不在本探针范围。

跑法：python scripts/probe_layer_eye_rename.py
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402

FIX = os.path.join(ROOT, "probe_layer_eye_rename.png")
GID = 1

OPS = [
    "layers-dump",                  # ck0 只有背景
    "layer-new", "layer-new", "layer-new",
    "multi-select:1,2",
    "layer-group",                  # GID 组含层 1/2
    "layers-dump",                  # ck1 成员 v=true ev=true
    "group-visible:%d,0" % GID,     # 隐藏组
    "layers-dump",                  # ck2 成员 v=false ev=true（核心）
    "eye:1,0",                      # 隐藏期间关掉成员1的自身意愿
    "layers-dump",                  # ck3 成员1 ev=false，成员2 ev=true
    "group-visible:%d,1" % GID,     # 显示组
    "layers-dump",                  # ck4 成员1 v=false，成员2 v=true
    "rename-at:2:按行改名",          # 问题3：按行下标改名（层2此时未选中）
    "layers-dump",                  # ck5 层2 名字已改
]


def by_index(dump, i):
    for x in dump:
        if x["i"] == i:
            return x
    return None


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

    dumps = parse_events(lines, "layers-dump")
    assert len(dumps) == 6, "应有 6 份 layers-dump，实得 %d" % len(dumps)
    ck = [d["layers"] for d in dumps]

    # ck1：组内成员显示态 = 合成态 = true
    for i in (1, 2):
        l = by_index(ck[1], i)
        assert l and l["g"] == GID, "层%d 应在组里" % i
        assert l["v"] is True and l["ev"] is True, \
            "ck1 层%d 应 v/ev 都 true: %s" % (i, l)

    # ck2：隐藏组 → 合成假、眼睛图标仍真
    for i in (1, 2):
        l = by_index(ck[2], i)
        assert l["v"] is False, "ck2 层%d 合成为 hidden" % i
        assert l["ev"] is True, \
            "ck2 层%d 眼睛图标应保持睁眼（ev=true）: %s" % (i, l)

    # ck3：隐藏期间点层1眼睛 → 自身意愿 false，图标变闭；层2不变
    l1 = by_index(ck[3], 1)
    l2 = by_index(ck[3], 2)
    assert l1["v"] is False and l1["ev"] is False, \
        "ck3 层1 自身意愿应为闭眼: %s" % l1
    assert l2["v"] is False and l2["ev"] is True, \
        "ck3 层2 应保持睁眼: %s" % l2

    # ck4：显示组 → 层1 保持隐藏（自身关），层2 恢复显示
    l1 = by_index(ck[4], 1)
    l2 = by_index(ck[4], 2)
    assert l1["v"] is False, "ck4 层1 按自身意愿保持隐藏: %s" % l1
    assert l2["v"] is True, "ck4 层2 应恢复显示: %s" % l2

    # ck5：按行下标改名（层2 未被选中），验证不依赖选中态
    l2 = by_index(ck[5], 2)
    assert l2["n"] == "按行改名", "ck5 层2 应按行下标改名: %s" % l2

    print("PASS: 组隐藏眼睛显示态保持 / 点眼睛改自身意愿 / 按行下标重命名", flush=True)


if __name__ == "__main__":
    main()
