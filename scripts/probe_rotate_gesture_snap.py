"""回归：真实旋柄指针路径转过 360°/720° 后，Shift 15° 与 ±3° 磁吸仍生效。

跟 probe_rotate_360_snap.py 的区别：那个走 Rust 脚本 op（角度直接传绝对值），
这个走 QML **真实手势**路径 —— `--probe-hand-rotate-path=mods:a0,a1,...,an`
按下旋柄后逐帧喂指针角，QML 取最短角差累加成累计角，再交给 Rust 吸附。
转过整圈后 atan2 会回绕，旧实现把「当前 - 按下」直接当角度，最终又变回 0°，
看起来就是「转到 360 之后角度吸附失效」。
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
    ImageDraw.Draw(im).rectangle([120, 90, 520, 390], fill=(200, 30, 30))
    im.save(FIXTURE)


def sweep(turns, tail):
    """指针角序列：整圈按 15° 步进，末尾再走 tail 度（不跨 ±180）。"""
    total = turns * 360
    pts = [0.0]
    a = 0.0
    while a < total - 1e-6:
        a = min(a + 15.0, total)
        pts.append(a)
    pts.append(total + tail)
    return pts


def run(paths):
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    args = [EXE, FIXTURE,
            "--script=extract:120,90,400,300",
            "--probe-rotate"]
    for mods, pts in paths:
        args.append("--probe-hand-rotate-path=%d:%s" % (
            mods, ",".join("%g" % p for p in pts)))
    p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    try:
        out, err = p.communicate(timeout=6)
    except subprocess.TimeoutExpired:
        p.kill()
        out, err = p.communicate()
    events = []
    for line in out.decode("utf-8", "replace").splitlines():
        # mark 的 name 里嵌了 transformValuesJson 的裸引号，整行不是合法 JSON。
        if '"name":"hand-rotate-path:' not in line:
            continue
        events.append(line)
    return events


def main():
    make_fixture()
    cases = [
        # (整圈数, 尾部增量, mods, 期望累计角)
        (1, 14, 0,           374),
        (1, 14, MOD_MAGNET,  375),
        (1, 14, MOD_SHIFT,   375),
        (2, 14, MOD_MAGNET,  735),
        (2, 14, MOD_SHIFT,   735),
    ]
    ok = True
    for turns, tail, mods, expect in cases:
        pts = sweep(turns, tail)
        events = run([(mods, pts)])
        if not events:
            print("FAIL turns=%d mods=0x%08x: 没有 hand-rotate-path 事件" % (turns, mods))
            ok = False
            continue
        name = events[-1]
        cum = float(name.split("cumulative=")[1].split()[0])
        rot = float(name.split('"rot":')[1].split(",")[0])
        raw = turns * 360 + tail
        # cumulative 是未吸附的指针累计角；rot 才是落定角（受 Shift/磁吸影响）。
        good = abs(cum - raw) < 0.2 and abs(rot - expect) < 0.2
        print("%-4s turns=%d mods=0x%08x cumulative=%.1f rot=%.1f (期望 %d)" % (
            "PASS" if good else "FAIL", turns, mods, cum, rot, expect))
        ok = ok and good
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
