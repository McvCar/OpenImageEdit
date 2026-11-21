"""画布选中折叠组内的图层时，面板要自动展开组并定位到该行。

用户报的问题：点画布选中的图层如果藏在折叠组里，列表里根本没有它的行，
revealLayerRow 直接放弃，用户看不到选的是谁。现在会沿父组链展开所有折叠
祖先再定位。折叠是纯视图状态，不进撤销栈、不动像素。

跑法：
    python scripts/probe_reveal_group.py
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

FIX = os.path.join(ROOT, "probe_reveal_group.png")
SHOT = os.path.join(ROOT, "probe_reveal_group_window.png")
GID = 1

OPS = [
    "layer-new", "layer-new", "layer-new",
    "multi-select:1,2",
    "layer-group",                      # gid = 1，成员 index 1 / 2
    "group-collapse:%d,1" % GID,        # 折叠
    "select-layer:0",                   # 先把主选中层挪出组，确保后续有 changed
    "groups-dump",                      # ck0：c = true
    "select-layer:1",                   # 画布选中组内图层
    "groups-dump",                      # ck1：c = false（自动展开）
    "group-collapse:%d,1" % GID,
    "select-layer:0",
    "select-layer:2",                   # 另一个成员也要触发
    "groups-dump",                      # ck2：c = false
]


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
    # 留一张面板截图，肉眼确认展开后成员行可见且滚动到了选中行。
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

    dumps = parse_events(lines, "groups-dump")
    assert len(dumps) >= 3, "缺少 groups-dump（共 %d 次）" % len(dumps)
    for n in range(3):
        groups = dumps[n]["groups"]
        assert len(groups) == 1, "预期只有 1 个组"
    assert dumps[0]["groups"][0]["c"] is True, "前置条件：组要处于折叠态"
    assert dumps[1]["groups"][0]["c"] is False, "选中成员后必须自动展开组"
    assert dumps[2]["groups"][0]["c"] is False, "再次折叠后选另一成员也要展开"
    print("PASS  画布选层自动展开折叠组")


if __name__ == "__main__":
    main()
