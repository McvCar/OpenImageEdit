"""验证「拖旋柄不会越转越糊」。

思路：同一张高频条纹图，用三条路径各转到 37°，再互相比较。

  A  一次性转到 37°（基准：只重采样一次，等价于理想结果）
  B  旋柄拖两次（18.5° + 18.5°）——应当与 A 逐像素相同
  C  角度对话框改两次（18.5° + 18.5°）——每次都是一次独立重采样，必然掉细节

只要 B 与 A 的 RMSE 为 0、且 C 明显差于 A，就说明旋柄走的是「始终从原图
出发」的路子，来回拖多少次都不会累积模糊。

用法：python scripts/verify_rotate.py [exe]        （默认 target/debug/imageedit.exe）
"""

import os
import json
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw, ImageStat

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "debug", "imageedit.exe")
TMP = tempfile.gettempdir()
# Qt 的 DLL 不在 PATH 里时 exe 起不来。
QT_BIN = r"D:\Qt\6.8.3\msvc2022_64\bin"

def fixture(path):
    """4px 周期的黑白竖条：任何一次重采样都会明显削掉对比度。"""
    im = Image.new("RGB", (1200, 900), (255, 255, 255))
    d = ImageDraw.Draw(im)
    for x in range(0, 1200, 4):
        d.rectangle([x, 0, x + 1, 899], fill=(0, 0, 0))
    im.save(path)

def run(src, ops, dst):
    import shutil
    shutil.copyfile(src, dst)
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    script = "extract:100,100,1000,800;" + ops + ";save"
    out = subprocess.run(
        [EXE, dst, "--script=" + script, "--auto-quit"],
        capture_output=True, text=True, env=env, timeout=180,
    )
    if out.returncode != 0:
        raise SystemExit(f"exe 退出码 {out.returncode}\n{out.stdout[-800:]}\n{out.stderr[-800:]}")

BOX = (250, 250, 950, 650)  # 避开旋转后留下的空角

def rmse(a, b):
    d = ImageChops.difference(a.crop(BOX), b.crop(BOX))
    st = ImageStat.Stat(d)
    return (sum(v * v for v in st.rms) / len(st.rms)) ** 0.5


def layer_infos(img, ops):
    """跑一段脚本，把其中每条 layer-info 解析成 dict 返回。"""
    env = dict(os.environ)
    env["PATH"] = QT_BIN + os.pathsep + env.get("PATH", "")
    out = subprocess.run(
        [EXE, img, "--script=" + ops, "--auto-quit"],
        capture_output=True, text=True, env=env, timeout=180,
    )
    return [
        json.loads(line)
        for line in out.stdout.splitlines()
        if '"event":"layer-info"' in line
    ]


def check_no_drift(src):
    """★11-08 回归：反复拖旋柄（净角度回到 0）不许把图层挪走。

    旧实现落定时用 `offset + 位图尺寸/2` 反推中心，位图摆放本身是
    `round(center - w/2)`，于是每转一轮中心被量化一次；外接框宽高奇偶交替
    让量化方向跟着翻转，转两三次就肉眼可见地漂 1px（用户报的 bug）。
    """
    ops = (
        "extract:100,100,1000,800;layer-info:0,base;"
        + "rotate-handle:0,9,1;rotate-handle:0,-9,1;" * 3
        + "layer-info:0,after"
    )
    infos = layer_infos(src, ops)
    if len(infos) < 2:
        return ["读不到 layer-info（旋转漂移回归没法判定）"]
    a, b = infos[0], infos[-1]
    fails = []
    for key in ("cx", "cy"):
        if abs(a[key] - b[key]) > 1e-6:
            fails.append(f"三轮旋柄往返后中心 {key} 漂移：{a[key]:.3f} → {b[key]:.3f}")
    for key in ("ox", "oy"):
        if a[key] != b[key]:
            fails.append(f"三轮旋柄往返后位置 {key} 漂移：{a[key]} → {b[key]}")
    print(
        f"  三轮旋柄往返后的中心/位置："
        f"({a['cx']:.3f},{a['cy']:.3f})@{a['ox']},{a['oy']} → "
        f"({b['cx']:.3f},{b['cy']:.3f})@{b['ox']},{b['oy']}"
    )
    return fails

def contrast(p):
    return ImageStat.Stat(p.crop(BOX)).stddev[0]

def main():
    if not os.path.exists(EXE):
        raise SystemExit(f"找不到 {EXE}，先 cargo build -p imageedit-ui")
    src = os.path.join(TMP, "rotfix_src.png")
    fixture(src)

    a, b, c, d = (os.path.join(TMP, f"rotfix_{k}.png") for k in "abcd")
    run(src, "rotate-layer:0,37", a)
    run(src, "rotate-handle:0,18.5;rotate-handle:0,18.5", b)
    run(src, "rotate-layer:0,18.5;rotate-layer:0,18.5", c)
    # 对照组：「不扩展」选项会裁剪，只能烘焙进位图，两次必然掉细节 ——
    # 用它证明这套 RMSE 指标确实能测出累积重采样。
    run(src, "rotate-layer:0,18.5,0;rotate-layer:0,18.5,0", d)

    A, B, C, D = (Image.open(p).convert("L") for p in (a, b, c, d))
    fails = []

    r_ab = rmse(A, B)
    print(f"  旋柄拖两次  vs 一次 37°：RMSE={r_ab:.2f}   对比度 {contrast(A):.1f} → {contrast(B):.1f}")
    if r_ab > 0.01:
        fails.append(f"旋柄两次拖拽与单次旋转不一致（RMSE={r_ab:.2f}），原图复用失效")

    r_ac = rmse(A, C)
    print(f"  连续改两次角度 vs 一次 37°：RMSE={r_ac:.2f}   对比度 {contrast(A):.1f} → {contrast(C):.1f}")
    if r_ac > 0.01:
        fails.append(f"连续改两次角度与单次旋转不一致（RMSE={r_ac:.2f}），参数化落定失效")

    ca, cd = contrast(A), contrast(D)
    r_ad = rmse(A, D)
    print(f"  [对照] 不扩展连转两次 vs 一次 37°：RMSE={r_ad:.2f}   对比度 {ca:.1f} → {cd:.1f}")
    if r_ad < 1.0 or cd > ca * 0.98:
        fails.append("对照组的累积重采样没有掉细节，说明指标本身失效（测试有问题）")

    fails.extend(check_no_drift(src))

    if fails:
        for f in fails:
            print("  FAIL  " + f)
        return 1
    print("ALL PASS")
    return 0

if __name__ == "__main__":
    sys.exit(main())
