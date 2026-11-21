"""真实 PSD：嵌套折叠组内层被选中时的列表定位验证。

用户报告：0302-界面UI-1.psd（637 层 / 197 组）里选中未展开的图层，
列表定位不准。根因是展开组后 Repeater delegate / contentHeight 要到下一帧
才建好，用它 clamp 会把滚动位置压回旧范围。

用 --probe-reveal=626 在 QML 就绪后（脚本路径太早）走真实选层回调，
上报目标显示行 / contentY / 视口高度，断言目标行落在可见区。

跑法：
    python scripts/probe_reveal_realpsd_pos.py
"""
import json
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402
import shot_window  # noqa: E402

PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"
SHOT = os.path.join(ROOT, "probe_reveal_realpsd_after.png")
TARGET = 626   # 组3(折叠) → 组4(展开) → 组10(折叠)
PITCH = 44 + 3


def snapshot(proc, path):
    hwnd = shot_window.find_main_window(proc.pid)
    if not hwnd:
        return
    shot_window.user32.SetForegroundWindow(hwnd)
    time.sleep(0.4)
    sys.argv = ["shot_window", str(proc.pid), path]
    shot_window.main()


def main():
    env = dict(os.environ)
    env["QT_FORCE_STDERR_LOGGING"] = "1"
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    proc = subprocess.Popen(
        [EXE, PSD, "--probe-layers", "--probe-panel-rows",
         "--probe-reveal=%d" % TARGET],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, 'reveal-report:', timeout=300)
    time.sleep(0.5)
    snapshot(proc, SHOT)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    reports = []
    for ln in lines:
        if 'reveal-report:idx=' not in ln:
            continue
        at = ln.find('reveal-report:idx=')
        payload = ln[at + len('reveal-report:'):].split('"', 1)[0]
        kv = dict(p.split("=", 1) for p in payload.split(",") if "=" in p)
        try:
            reports.append({k: int(v) for k, v in kv.items()})
        except ValueError:
            continue

    assert reports, "没有拿到 reveal-report（定位回调未触发）"
    final = reports[-1]
    row, cy, vh = final["row"], final["cy"], final["vh"]
    y0 = row * PITCH
    y1 = y0 + 44
    print("目标层 i=%d 显示行=%d contentY=%d 视口=%d 共%d行" %
          (TARGET, row, cy, vh, final["rows"]))
    print("目标行区间 [%d, %d] 可见区 [%d, %d]" % (y0, y1, cy, cy + vh))
    assert row >= 0, "目标层没有出现在面板显示行里（祖先组没展开）"
    assert y1 > cy and y0 < cy + vh, "目标行不在可见区内 —— 定位不准"
    print("PASS  真实 PSD 折叠组内层选中后自动展开并精确定位")


if __name__ == "__main__":
    main()
