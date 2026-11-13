#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""PSD 特性画像 + 保真差距 + 瓦片成本估算（只读，独立于本项目的解码器）。

用法::

    # 只看特性画像与瓦片成本
    python scripts/psd_fidelity_report.py path/to/file.psd

    # 再加上「本项目的重建合成」与「PS 自带合并图」的差距
    python scripts/psd_fidelity_report.py path/to/file.psd --flat tmp/flat.png

`--flat` 的输入由应用侧导出::

    imageedit.exe file.psd --probe-layers --script=dump-flat   # 写 tmp/flat.png

依赖 psd-tools（`pip install psd-tools`）+ numpy + pillow。

输出四段：
  [1] 文档级：尺寸 / 位深 / 色彩模式
  [2] 图层与分组统计：类型、混合模式、不透明度、显隐
  [3] 需要合成语义的特性：剪贴蒙版 / 图层蒙版 / 组蒙版 / 图层样式 / 调整层
  [4] 瓦片成本：不同 tile 尺寸下「脏块重合成」的估算耗时（按全幅重建实测值折算）
可选 [5]：per-block MAE 与最差区域涉及的图层（归因）

`--ms-full` 传全幅重建的实测毫秒数（默认 124，见文档 §2.3），用于折算。
"""

import argparse
import collections
import statistics
import sys

import numpy as np

try:  # Python 3.7+
    sys.stdout.reconfigure(encoding="utf-8")
except Exception:
    pass

import warnings

warnings.filterwarnings("ignore")
from psd_tools import PSDImage  # noqa: E402

ADJUSTMENT_KINDS = {
    "brightnesscontrast",
    "huesaturation",
    "levels",
    "curves",
    "colorbalance",
    "blackandwhite",
    "gradientmap",
    "photoFilter",
    "channelmixer",
    "invert",
    "posterize",
    "threshold",
    "exposure",
    "vibrance",
    "selectivecolor",
    "colorlookup",
}


def collect(psd):
    stats = {
        "layers": 0,
        "groups": 0,
        "max_depth": 0,
        "hidden": 0,
        "kinds": collections.Counter(),
        "blend": collections.Counter(),
        "opacity": 0,
        "clip": 0,
        "mask": 0,
        "mask_black": 0,
        "vector_mask": 0,
        "effects": collections.Counter(),
        "effect_layers": 0,
        "adjust": collections.Counter(),
        "group_blend": collections.Counter(),
        "group_mask": 0,
        "group_opacity": 0,
        "boxes": [],
    }

    def walk(nodes, depth):
        stats["max_depth"] = max(stats["max_depth"], depth)
        for node in nodes:
            if node.is_group():
                stats["groups"] += 1
                stats["group_blend"][str(node.blend_mode)] += 1
                if int(node.opacity) != 255:
                    stats["group_opacity"] += 1
                if node.mask is not None:
                    stats["group_mask"] += 1
                if not node.visible:
                    stats["hidden"] += 1
                walk(node, depth + 1)
                continue

            stats["layers"] += 1
            stats["kinds"][node.kind] += 1
            stats["blend"][str(node.blend_mode)] += 1
            if int(node.opacity) != 255:
                stats["opacity"] += 1
            if not node.visible:
                stats["hidden"] += 1
                continue
            if node.clipping:
                stats["clip"] += 1
            if node.mask is not None:
                stats["mask"] += 1
                try:
                    if not node.mask.background_color:
                        stats["mask_black"] += 1
                except Exception:
                    pass
            if getattr(node, "vector_mask", None) is not None:
                stats["vector_mask"] += 1
            if node.effects:
                stats["effect_layers"] += 1
                for e in node.effects:
                    stats["effects"][e.__class__.__name__] += 1
            if node.kind in ADJUSTMENT_KINDS:
                stats["adjust"][node.kind] += 1

            l, t, r, b = node.bbox
            l, t = max(0, l), max(0, t)
            r, b = min(psd.width, r), min(psd.height, b)
            if r > l and b > t:
                stats["boxes"].append((l, t, r, b))

    walk(psd, 0)
    return stats


def tile_table(stats, width, height, ms_full, tiles=(32, 64, 128, 256)):
    boxes = stats["boxes"]
    total_area = sum((r - l) * (b - t) for l, t, r, b in boxes) or 1
    rows = []
    for size in tiles:
        cols = (width + size - 1) // size
        lines = (height + size - 1) // size
        areas = []
        for ty in range(lines):
            for tx in range(cols):
                x0, y0 = tx * size, ty * size
                x1, y1 = min(width, x0 + size), min(height, y0 + size)
                s = 0
                for l, t, r, b in boxes:
                    iw = max(0, min(r, x1) - max(l, x0))
                    ih = max(0, min(b, y1) - max(t, y0))
                    if iw and ih:
                        s += iw * ih
                areas.append(s)
        areas.sort()
        q = lambda p: areas[min(len(areas) - 1, int(len(areas) * p))]
        to_ms = lambda a: a / total_area * ms_full
        rows.append(
            (
                size,
                len(areas),
                to_ms(q(0.5)),
                to_ms(q(0.95)),
                to_ms(areas[-1]),
            )
        )
    return total_area, rows


def mae_report(psd, flat_path, block):
    from PIL import Image

    merged = psd.topil().convert("RGBA")
    if merged.size != (psd.width, psd.height):
        merged = merged.resize((psd.width, psd.height), Image.NEAREST)
    ours = Image.open(flat_path).convert("RGBA")
    if ours.size != merged.size:
        ours = ours.resize(merged.size, Image.NEAREST)

    a = np.asarray(merged).astype(np.int16)
    b = np.asarray(ours).astype(np.int16)
    diff = np.abs(a[..., :3] - b[..., :3]).max(axis=2)
    alpha = np.abs(a[..., 3] - b[..., 3])
    print("\n[5] 重建合成 vs PS 自带合并图")
    print(f"  MAE(最大通道)={diff.mean():.3f}  p99={np.percentile(diff, 99):.0f}  max={diff.max()}")
    print(f"  差>8 的像素 {(diff > 8).mean() * 100:.2f}%   差>32 的像素 {(diff > 32).mean() * 100:.2f}%")
    print(f"  alpha MAE={alpha.mean():.3f}")

    h, w = diff.shape
    blocks = []
    for y in range(h // block):
        for x in range(w // block):
            blk = diff[y * block:(y + 1) * block, x * block:(x + 1) * block]
            blocks.append((float(blk.mean()), x * block, y * block))
    blocks.sort(reverse=True)
    print(f"\n  最差 {min(8, len(blocks))} 个 {block}px 块:")
    for m, x, y in blocks[:8]:
        print(f"    {m:7.2f}  ({x:4d},{y:4d})")
    return blocks


def attribution(psd, x0, y0, size):
    print(f"\n  块 ({x0},{y0})-({x0 + size},{y0 + size}) 内可见图层:")
    stack = list(psd)
    hits = []
    while stack:
        node = stack.pop()
        if node.is_group():
            stack.extend(list(node))
            continue
        if not node.visible:
            continue
        l, t, r, b = node.bbox
        if r > x0 and l < x0 + size and b > y0 and t < y0 + size:
            hits.append(node)
    for node in hits:
        eff = [e.__class__.__name__ for e in (node.effects or [])]
        print(
            f"    {node.name[:26]:28s} {node.kind:16s} clip={int(bool(node.clipping))} "
            f"mask={int(node.mask is not None)} blend={str(node.blend_mode).split('.')[-1]:12s} "
            f"op={node.opacity:3d} fx={','.join(eff) if eff else '-'}"
        )
    if not hits:
        print("    （无）")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("psd")
    ap.add_argument("--flat", help="本项目重建合成导出的 PNG（dump-flat）")
    ap.add_argument("--block", type=int, default=32, help="MAE 分块大小，默认 32")
    ap.add_argument("--tile", type=int, default=0, help="只看某一个 tile 尺寸")
    ap.add_argument("--ms-full", type=float, default=124.0, help="全幅重建实测毫秒（默认 124）")
    ap.add_argument("--attribute", type=int, default=3, help="对最差 N 个块做图层归因")
    args = ap.parse_args()

    psd = PSDImage.open(args.psd)
    stats = collect(psd)

    print(f"[1] {args.psd}")
    hdr = psd._record.header
    print(f"  {psd.width}x{psd.height}  depth={hdr.depth}  mode={hdr.color_mode}  channels={hdr.channels}")

    print(f"\n[2] 图层 {stats['layers']} / 组 {stats['groups']} / 最深嵌套 {stats['max_depth']} / 隐藏 {stats['hidden']}")
    print("  类型: " + "  ".join(f"{k}={v}" for k, v in stats["kinds"].most_common()))
    print("  混合: " + "  ".join(f"{k.split('.')[-1]}={v}" for k, v in stats["blend"].most_common()))
    if stats["opacity"]:
        print(f"  不透明度<100%: {stats['opacity']}")

    print("\n[3] 需要合成语义的特性（可见层）")
    print(f"  剪贴蒙版: {stats['clip']}    图层蒙版: {stats['mask']}（黑底 {stats['mask_black']}）"
          f"    矢量蒙版: {stats['vector_mask']}")
    print(f"  带样式的层: {stats['effect_layers']}，样式实例 {sum(stats['effects'].values())}: "
          + "  ".join(f"{k}={v}" for k, v in stats["effects"].most_common()))
    print(f"  调整层: {sum(stats['adjust'].values())}  " + "  ".join(f"{k}={v}" for k, v in stats["adjust"].most_common()))
    print(f"  分组: " + "  ".join(f"{k.split('.')[-1]}={v}" for k, v in stats["group_blend"].most_common())
          + f"  组蒙版={stats['group_mask']}  组不透明度<100%={stats['group_opacity']}")

    tiles = (args.tile,) if args.tile else (32, 64, 128, 256)
    total_area, rows = tile_table(stats, psd.width, psd.height, args.ms_full, tiles)
    print(f"\n[4] 瓦片成本（图层面积合计 {total_area / 1e6:.1f} Mpx；全幅重建 {args.ms_full:.0f} ms 折算）")
    print("  tile   块数    p50       p95       max")
    for size, n, p50, p95, mx in rows:
        print(f"  {size:4d} {n:6d}  {p50:7.2f}ms {p95:7.2f}ms {mx:7.2f}ms")
    print("  （尺寸越小越省，但块数多；本项目建议 64）")

    if args.flat:
        blocks = mae_report(psd, args.flat, args.block)
        for m, x, y in blocks[: max(0, args.attribute)]:
            attribution(psd, x, y, args.block)


if __name__ == "__main__":
    main()
