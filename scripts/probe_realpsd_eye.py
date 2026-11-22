"""真实 PSD 验证「组隐藏时成员眼睛保持睁眼」。

两阶段：
  1) groups-dump + layers-dump，挑面板顶部附近一个已展开、成员可见的组；
  2) 重新启动，group-visible:<gid>,0 隐藏该组，截图窗口，肉眼核对
     组行闭眼、成员行保持睁眼。

跑法：
    python scripts/probe_realpsd_eye.py
"""
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

import shot_window  # noqa: E402
from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"
SHOT = os.path.join(ROOT, "probe_realpsd_eye_window.png")


def run(ops, timeout=300):
    env = dict(os.environ)
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    proc = subprocess.Popen(
        [EXE, PSD, "--probe-layers", "--script=" + ";".join(ops)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=timeout)
    time.sleep(0.5)
    return proc, lines


def main():
    # 阶段 1：拿结构，挑目标组。
    proc, lines = run(["groups-dump", "layers-dump"])
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    gd = parse_events(lines, "groups-dump")
    ld = parse_events(lines, "layers-dump")
    if not gd or not ld:
        print("FAILED: 没有拿到 dump 事件")
        for line in lines[-15:]:
            print("  ", line.strip()[:200])
        return
    groups = gd[-1]["groups"]
    rows = ld[-1]["layers"]
    print("组数:", len(groups), "显示行数:", len(rows))

    # 全表扫描：找成员睁眼（ev=True）≥2 的组，取最靠顶部的一个。
    # 组标题行在面板里插在它第一个成员行之前，所以用第一个成员的
    # 显示行位置近似组标题位置。
    best = None
    by_gid = {}
    for pos, r in enumerate(rows):
        by_gid.setdefault(r.get("g", 0), []).append((pos, r))
    for g, members in by_gid.items():
        if g <= 0 or len(members) < 2:
            continue
        awake = [m for m in members if m[1].get("ev", True)]
        if len(awake) < 2:
            continue
        if best is None or awake[0][0] < best[1][0][0]:
            best = (g, awake)
    if not best:
        print("FAILED: 整个 PSD 没有成员睁眼 ≥2 的组")
        return
    gid, awake = best
    meta = next((x for x in groups if x["id"] == gid), None)
    first_pos = awake[0][0]
    print("目标组: id=%d 名称=%s 折叠=%s 首个成员行位=%d 睁眼成员=%d"
          % (gid, (meta or {}).get("n", ""), (meta or {}).get("c"),
             first_pos, len(awake)))
    for pos, m in awake[:6]:
        print("   成员 i=%d v=%s ev=%s n=%s"
              % (m["i"], m.get("v"), m.get("ev"), m.get("n", "")[:20]))

    # 阶段 2：选中首个成员触发 reveal（展开祖先 + 滚动定位）→
    # 隐藏该组 → dump → 截图。
    proc, lines = run([
        "select-layer:%d" % awake[0][1]["i"],
        "group-visible:%d,0" % gid,
        "layers-dump",
    ])
    hwnd = shot_window.find_main_window(proc.pid)
    if hwnd:
        shot_window.user32.SetForegroundWindow(hwnd)
        time.sleep(0.4)
        sys.argv = ["shot_window", str(proc.pid), SHOT]
        shot_window.main()
    else:
        print("WARN: 没找到窗口句柄，截图跳过")
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    dump = parse_events(lines, "layers-dump")[-1]["layers"]
    print("隐藏后状态（组 %d 及其成员）:" % gid)
    ok = True
    for pos, row in enumerate(dump[:14]):
        if row.get("gid") == gid or row.get("g") == gid:
            tag = "组行" if row.get("gid") == gid else "成员"
            print("  %s i=%s v=%s ev=%s n=%s"
                  % (tag, row.get("i"), row.get("v"), row.get("ev"),
                     row.get("n", row.get("name", ""))[:20]))
            if tag == "成员" and not row.get("ev", True):
                ok = False
    # 组行自身：标题行 v 应为 False（或 sv=False），成员 ev 必须 True。
    print("PASS" if ok else "FAILED: 有成员 ev 被改成 False")


if __name__ == "__main__":
    main()
