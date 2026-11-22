"""亚像素量「拖动期图层边缘」：逐帧算洋红强度加权的包围盒。

probe_drag_size 用阈值 bbox，只能看到 1px 级别的整格跳动；用户说的「缩小
一点点」很可能是**边缘半透明混叠**（双线性降采样的最外一行变成 50% 洋红，
肉眼就是「小了一圈」）。这里按 (R-G) 强度做面积加权，得到亚像素级宽度：

    width_sub = sum(weight) / max_weight

比阈值 bbox 灵敏一个数量级。

usage: python scripts/analyze_drag_edge.py <dir>
"""
from __future__ import annotations

import glob
import os
import re
import sys

import numpy as np
from PIL import Image


def load(path):
    im = Image.open(path).convert("RGB")
    a = np.asarray(im).astype(np.float32)
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    # 洋红度：红色与蓝色都高、绿色低。用 min(r-g, b-g) 作强度。
    w = np.minimum(r - g, b - g)
    w[w < 0] = 0.0
    # 硬闸门：只有"像洋红"的像素参与（暗帧、UI 灰底的微弱偏色一律排除）。
    hard = (r > 150) & (b > 150) & (g < 130)
    return np.where(hard, w, 0.0) / 255.0


def measure(w):
    tot = float(w.sum())
    if tot < 50.0:
        return None
    ys, xs = np.nonzero(w > 0.02)
    if len(xs) == 0:
        return None
    y0, y1, x0, x1 = int(ys.min()), int(ys.max()) + 1, int(xs.min()), int(xs.max()) + 1
    sub = w[y0:y1, x0:x1]
    # 面积 = 强度积分 / 峰值（峰值取 99.5 分位，避开个别过冲像素）。
    # 双线性把最外一行糊成半透明时，bbox 不变但**面积变小** —— 这正是肉眼
    # 看到的"缩小一点点"，所以面积比 bbox 更能说明问题。
    peak = float(np.percentile(sub, 99.5))
    peak = peak if peak > 0.05 else 1.0
    # 亚像素跨度：穿过方块中线的行/列，按两端像素的覆盖率反推边缘位置
    # （左边缘 = L + (1 - cov)，右边缘 = R + cov）。
    mid_y, mid_x = (y0 + y1) // 2, (x0 + x1) // 2
    row, col = w[mid_y], w[:, mid_x]
    rx = np.nonzero(row > 0.02)[0]
    ry = np.nonzero(col > 0.02)[0]
    span_x = span_y = 0.0
    if len(rx) and len(ry):
        l, r = int(rx.min()), int(rx.max())
        t, b = int(ry.min()), int(ry.max())
        span_x = (r + float(row[r])) - (l + 1.0 - float(row[l]))
        span_y = (b + float(col[b])) - (t + 1.0 - float(col[t]))
    return dict(box=(x0, y0, x1, y1), mass=tot, peak=peak,
                area_px=tot / peak, span=(span_x, span_y))


def main() -> int:
    d = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.environ.get("TEMP", "."), "probe_drag_size")
    files = sorted(glob.glob(os.path.join(d, "f*.png")),
                   key=lambda p: int(re.search(r"_(\d+)\.png$", p).group(1)))
    if not files:
        print("no frames in", d)
        return 1
    rows = []
    for p in files:
        w = load(p)
        m = measure(w)
        if m is None:
            continue
        ms = int(re.search(r"_(\d+)\.png$", p).group(1))
        rows.append((ms, m))
    if not rows:
        print("all frames empty")
        return 1
    # 按 x0 分成两组：静止组（x0 最小）与拖动组（x0 明显偏移）
    x0s = [r[1]["box"][0] for r in rows]
    base = min(x0s)
    still = [r for r in rows if abs(r[1]["box"][0] - base) <= 2]
    moved = [r for r in rows if r[1]["box"][0] - base > 20]
    print("still frames: %d  moved frames: %d" % (len(still), len(moved)))

    def stat(group, name):
        if not group:
            print(name, "-> none")
            return None
        areas = [r[1]["area_px"] for r in group]
        spans = [r[1]["span"] for r in group]
        ws = [r[1]["box"][2] - r[1]["box"][0] for r in group]
        hs = [r[1]["box"][3] - r[1]["box"][1] for r in group]
        print("%s: bbox %dx%d  area(px) mean=%.1f min=%.1f max=%.1f  "
              "span=%.2fx%.2f" % (
                  name, ws[len(ws) // 2], hs[len(hs) // 2],
                  float(np.mean(areas)), min(areas), max(areas),
                  float(np.mean([s[0] for s in spans])),
                  float(np.mean([s[1] for s in spans]))))
        return float(np.mean(areas)), float(np.mean([s[0] for s in spans]))

    a_still = stat(still, "静止")
    a_moved = stat(moved, "拖动")
    if a_still and a_moved:
        d_area = (a_moved[0] - a_still[0]) / a_still[0] * 100.0
        d_span = a_moved[1] - a_still[1]
        print("面积差 %+.3f%%  亚像素宽差 %+.2fpx（负 = 拖动期更小 = 用户说的缩小）"
              % (-d_area, d_span))
        print("RESULT", "SHRINK" if (d_area < -0.2 or d_span < -0.2) else "SAME")
    return 0


if __name__ == "__main__":
    sys.exit(main())
