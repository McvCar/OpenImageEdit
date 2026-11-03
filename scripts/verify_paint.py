"""断言 paint_probe.sh 产出的图，逐项检查修复 / 油漆 / 旋转的像素结果。"""
import sys
from PIL import Image

T = "C:/Users/Admin/AppData/Local/Temp"
src = Image.open(f"{T}/paint_in.png").convert("RGB")
S = src.load()


def load(name):
    im = Image.open(f"{T}/{name}").convert("RGB")
    return im, im.load()


def px(p, x, y):
    return p[x, y]


def near(a, b, tol):
    return all(abs(int(x) - int(y)) <= tol for x, y in zip(a, b))


fails = []


def check(label, ok, detail=""):
    print(("  PASS  " if ok else "  FAIL  ") + label + ("  " + detail if detail else ""))
    if not ok:
        fails.append(label)


print("[1] 油漆桶：连续填充，红色圆盘与条纹不受影响")
im, p = load("p_bucket.png")
check("背景 (5,5) 变蓝", near(px(p, 5, 5), (0, 0, 255), 6), str(px(p, 5, 5)))
check("圆外背景 (100,100) 也变蓝", near(px(p, 100, 100), (0, 0, 255), 6), str(px(p, 100, 100)))
check("圆盘中心 (60,60) 仍是红", near(px(p, 60, 60), (255, 0, 0), 6), str(px(p, 60, 60)))
check("条纹 (133,40) 仍是黑", near(px(p, 133, 40), (0, 0, 0), 8), str(px(p, 133, 40)))
check("黑点 (150,150) 不变", near(px(p, 150, 150), (0, 0, 0), 8), str(px(p, 150, 150)))

print("[2] 线性渐变：左红右蓝，中间过渡")
im, p = load("p_grad.png")
l, m, r = px(p, 2, 100), px(p, 100, 100), px(p, 197, 100)
check("左端偏红", l[0] > 200 and l[2] < 60, str(l))
check("右端偏蓝", r[2] > 200 and r[0] < 60, str(r))
check("中点混合", 80 < m[0] < 180 and 80 < m[2] < 180, str(m))

print("[3] 径向渐变（径向 + 80% 不透明）")
im, p = load("p_grad_rad.png")
c, mid, far = px(p, 100, 100), px(p, 100, 140), px(p, 100, 195)
check("圆心最亮", c[0] > 240, str(c))
check("向外变暗", mid[0] < c[0] and far[0] < mid[0], f"{c} {mid} {far}")

print("[4] 污点修复：黑点被周边浅灰吃掉")
im, p = load("p_spot.png")
v = px(p, 150, 150)
check("黑点中心变亮", v[0] > 180, str(v))

print("[5] 修复画笔：从条纹取样，灰带出现明暗起伏")
im, p = load("p_heal.png")
# x 110..139 才是一段干净灰带；再往右就是测试图里的黑点。
row = [p[x, 150][0] for x in range(110, 140)]
spread = max(row) - min(row)
before = [S[x, 150][0] for x in range(110, 140)]
check("原图该带是纯灰", max(before) - min(before) < 4, f"spread={max(before) - min(before)}")
check("修复后出现条纹起伏", spread > 60, f"spread={spread}")

print("[6] 修补：把黑点所在区域整体偏移取样后抹平")
im, p = load("p_patch.png")
v = px(p, 150, 150)
check("黑点区域被灰覆盖", v[0] > 200, str(v))

print("[7] 图层旋转 30°：红弧位置改变")
im, p = load("p_rot.png")
reds_src = [(x, y) for y in range(0, 200, 2) for x in range(0, 200, 2) if S[x, y][0] > 200 and S[x, y][1] < 60]
reds_dst = [(x, y) for y in range(0, 200, 2) for x in range(0, 200, 2) if p[x, y][0] > 200 and p[x, y][1] < 60]
cs = (sum(x for x, _ in reds_src) / len(reds_src), sum(y for _, y in reds_src) / len(reds_src))
cd = (sum(x for x, _ in reds_dst) / len(reds_dst), sum(y for _, y in reds_dst) / len(reds_dst))
moved = ((cd[0] - cs[0]) ** 2 + (cd[1] - cs[1]) ** 2) ** 0.5
check("红色像素质心移动", moved > 2.0, f"src={cs[0]:.1f},{cs[1]:.1f} dst={cd[0]:.1f},{cd[1]:.1f} moved={moved:.1f}px")

print("[8] 撤销：油漆桶 + undo 应回到原图")
im, p = load("p_undo.png")
diff = sum(1 for y in range(200) for x in range(200) if p[x, y] != S[x, y])
check("像素与原图一致", diff == 0, f"diff={diff}")

print("[9] 选区约束：只在 y>=120 的选区里填色")
im, p = load("p_sel.png")
check("选区内 (5,150) 变蓝", near(px(p, 5, 150), (0, 0, 255), 6), str(px(p, 5, 150)))
check("选区外 (5,50) 保持灰", near(px(p, 5, 50), (240, 240, 240), 6), str(px(p, 5, 50)))

print()
if fails:
    print(f"FAILED {len(fails)}: {fails}")
    sys.exit(1)
print("ALL PASS")
