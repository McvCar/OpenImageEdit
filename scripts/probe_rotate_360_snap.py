"""回归：旋柄累计转过 360°/720° 后，Shift 15° 硬吸与 ±3° 磁吸仍须生效。

用 --script= 的 rotate-snap op 跑多轮：
  extract 出可旋转图层 -> 先转到 pre 角 -> 再给一个「离 15° 倍数 2°」的增量，
断言：无修饰键时原样落定，Shift / 磁吸时吸到 15° 倍数。
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, "target", "debug", "imageedit.exe")
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = os.path.join(ROOT, "target", "probe-rot360")
FIXTURE = os.path.join(TMP, "fixture.png")

MOD_MAGNET = 0x10000000
MOD_SHIFT = 0x02000000


def make_fixture():
    from PIL import Image, ImageDraw
    os.makedirs(TMP, exist_ok=True)
    im = Image.new("RGB", (900, 700), (240, 240, 240))
    d = ImageDraw.Draw(im)
    d.rectangle([120, 90, 520, 390], fill=(200, 30, 30))
    im.save(FIXTURE)


def run(ops):
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    out = subprocess.run(
        [EXE, FIXTURE, "--script=" + ops, "--auto-quit"],
        capture_output=True, text=True, env=env, timeout=180,
    )
    if out.returncode != 0:
        raise SystemExit("exe 退出码 %d\n%s\n%s" % (
            out.returncode, out.stdout[-1200:], out.stderr[-1200:]))
    events = []
    for line in out.stdout.splitlines():
        if '"event":"rotate-snap"' in line:
            events.append(json.loads(line))
    return events


def main():
    make_fixture()
    cases = [
        (0,   14, 0,          14),
        (0,   14, MOD_MAGNET, 15),
        (0,   14, MOD_SHIFT,  15),
        (360, 14, 0,          374),
        (360, 14, MOD_MAGNET, 375),
        (360, 14, MOD_SHIFT,  375),
        (720, 14, MOD_MAGNET, 735),
        (720, 14, MOD_SHIFT,  735),
    ]
    ok = True
    for pre, delta, mods, expect in cases:
        script = ("extract:120,90,400,300;"
                  "rotate-snap:1,%d,0;rotate-snap:1,%d,%d;layer-info:1" % (
                      pre, delta, mods))
        ev = run(script)
        if len(ev) < 2:
            print("FAIL pre=%d delta=%d mods=%d: 缺少 rotate-snap 事件" % (pre, delta, mods))
            ok = False
            continue
        rot = ev[-1]["rot"]
        good = abs(rot - expect) < 0.2
        print("%-4s pre=%4d delta=%2d mods=0x%08x -> rot=%7.2f (期望 %d)" % (
            "PASS" if good else "FAIL", pre, delta, mods, rot, expect))
        ok = ok and good
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
