"""用用户给的真实 PSD 复现「选中折叠组内层，列表定位不准」。

加载 0302-界面UI-1.psd，dump 组/层结构，挑一个折叠组内的层，
折叠后选中它，截图看列表滚到了哪里。

跑法：
    python scripts/probe_reveal_realpsd.py
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))

from probe_layer_panel import EXE, parse_events, pump  # noqa: E402

PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"
OUT = os.path.join(ROOT, "probe_reveal_realpsd.json")


def main():
    env = dict(os.environ)
    env["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + env.get("PATH", "")
    proc = subprocess.Popen(
        [EXE, PSD, "--probe-layers", "--script=groups-dump;layers-dump"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        errors="replace", env=env, bufsize=1,
    )
    lines, _ = pump(proc, '"event":"script-done"', timeout=300)
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    gd = parse_events(lines, "groups-dump")
    ld = parse_events(lines, "layers-dump")
    if not gd or not ld:
        print("FAILED: 没有拿到 dump 事件")
        for line in lines[-20:]:
            print("  ", line.strip()[:200])
        return
    groups = gd[-1]["groups"]
    layers = ld[-1]["layers"]
    print("组数:", len(groups), "层数:", len(layers))
    collapsed = [g for g in groups if g.get("c")]
    print("折叠组数:", len(collapsed))
    for g in collapsed[:12]:
        print("  折叠组 id=%d depth=%d cnt=%d mids=%s" %
              (g["id"], g.get("d", 0), g.get("cnt", 0),
               str(g.get("mids", []))[:80]))
    for g in groups[:12]:
        print("  组 id=%d c=%s d=%d cnt=%d n=%s" %
              (g["id"], g.get("c"), g.get("d", 0), g.get("cnt", 0), g.get("n")))
    for l in layers[:12]:
        print("  层 i=%d id=%d g=%d n=%s" %
              (l["i"], l["id"], l.get("g", 0), l.get("n")))
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump({"groups": groups, "layers": layers}, f, ensure_ascii=False,
                  indent=1)
    print("saved", OUT)


if __name__ == "__main__":
    main()
