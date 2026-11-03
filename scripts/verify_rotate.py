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

def contrast(p):
    return ImageStat.Stat(p.crop(BOX)).stddev[0]

def main():
    if not os.path.exists(EXE):
        raise SystemExit(f"找不到 {EXE}，先 cargo build -p imageedit-ui")
    src = os.path.join(TMP, "rotfix_src.png")
    fixture(src)

    a, b, c = (os.path.join(TMP, f"rotfix_{k}.png") for k in "abc")
    run(src, "rotate-layer:0,37", a)
    run(src, "rotate-handle:0,18.5;rotate-handle:0,18.5", b)
    run(src, "rotate-layer:0,18.5;rotate-layer:0,18.5", c)

    A, B, C = (Image.open(p).convert("L") for p in (a, b, c))
    fails = []

    r_ab = rmse(A, B)
    print(f"  旋柄拖两次  vs 一次 37°：RMSE={r_ab:.2f}   对比度 {contrast(A):.1f} → {contrast(B):.1f}")
    if r_ab > 0.01:
        fails.append(f"旋柄两次拖拽与单次旋转不一致（RMSE={r_ab:.2f}），原图复用失效")

    ca, cc = contrast(A), contrast(C)
    r_ac = rmse(A, C)
    print(f"  连续改两次角度 vs 一次 37°：RMSE={r_ac:.2f}   对比度 {ca:.1f} → {cc:.1f}")
    if r_ac < 1.0 or cc > ca * 0.98:
        fails.append("连续两次独立重采样没有掉细节，说明 C 这组参照失效（测试本身有问题）")

    if fails:
        for f in fails:
            print("  FAIL  " + f)
        return 1
    print("ALL PASS")
    return 0

if __name__ == "__main__":
    sys.exit(main())
