"""拖入/拖出隐藏分组时的显隐同步回归。

覆盖用户报的问题：把图层拖进隐藏组后，图层没有跟随组变成隐藏；
撤销 / 重做、再拖出来时显隐也要一起回滚 / 还原。

跑法：
    python scripts/probe_move_group_state.py
输出：PASS / FAILED
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, fixture, parse_events, pump  # noqa: E402
import shot_window  # noqa: E402

FIX = os.path.join(ROOT, "probe_move_group_state.png")
SHOT = os.path.join(ROOT, "probe_move_group_state_window.png")
GID = 1

OPS = [
    "layers-dump",                    # ck0
    "layer-new", "layer-new", "layer-new",
    "multi-select:1,2",
    "layer-group",                    # gid = 1
    "group-visible:%d,0" % GID,       # 隐藏组（成员 1/2 变隐藏）
    "layers-dump",                    # ck1
    "move-group:3,%d" % GID,          # 把组外可见层拖进隐藏组
    "layers-dump",                    # ck2 层 3 必须隐藏
    "undo",
    "layers-dump",                    # ck3 撤销：层 3 显示且出组
    "redo",
    "layers-dump",                    # ck4 重做：层 3 隐藏且在组内
    "move-group:3,0",                 # 拖回顶层
    "layers-dump",                    # ck5 层 3 恢复显示
]


def dump_at(dumps, n):
    assert len(dumps) > n, "缺少第 %d 次 layers-dump（共 %d 次）" % (n + 1, len(dumps))
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
    lines, _ = pump(proc, '"event":"script-done"', timeout=60)
    time.sleep(0.5)
    # 顺手留一张面板截图，肉眼确认组行的三角 / 文件夹图标布局。
    hwnd = shot_window.find_main_window(proc.pid)
    if hwnd:
        shot_window.user32.SetForegroundWindow(hwnd)
        time.sleep(0.5)
        sys.argv = ["shot_window", str(proc.pid), SHOT]
        shot_window.main()
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    dumps = parse_events(lines, "layers-dump")
    ck0 = dump_at(dumps, 0)
    ck1 = dump_at(dumps, 1)
    ck2 = dump_at(dumps, 2)
    ck3 = dump_at(dumps, 3)
    ck4 = dump_at(dumps, 4)
    ck5 = dump_at(dumps, 5)

    assert len(ck0) == 1, "fixture 只有背景层"
    assert ck1[3]["v"] is True and ck1[3]["g"] == 0, "拖之前层 3 可见且在组外"
    assert ck2[3]["v"] is False and ck2[3]["g"] == GID, "拖进隐藏组必须跟着隐藏"
    assert ck3[3]["v"] is True and ck3[3]["g"] == 0, "撤销要同时还原显隐和归属"
    assert ck4[3]["v"] is False and ck4[3]["g"] == GID, "重做要重新隐藏"
    assert ck5[3]["v"] is True and ck5[3]["g"] == 0, "拖出隐藏组要恢复显示"
    print("PASS  隐藏组拖入 / 撤销 / 重做 / 拖出显隐全部同步")


if __name__ == "__main__":
    main()
