#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成一份「带嵌套组」的 PSD fixture（psd-tools 写，独立于我们的编码器）。

结构（自底向上）：

    背景 (raster, 不透明)
    组「Button」（展开）
        ├ 按钮底
        └ 按钮字
    组「外层」（文件里存为**关闭**态）
        ├ 组「内层」
        │   ├ 内层 A
        │   └ 内层 B
        └ 外层散层

用途：验证 `decode_psd_layers` 把 PSD 的文件夹树还原成 `PsdGroupData`
（父子关系 / 折叠态 / 成员归属），以及面板缩进与折叠。

    python scripts/make_psd_groups_fixture.py [--out DIR]
"""
import argparse
import os
import sys

from PIL import Image
from psd_tools import PSDImage
from psd_tools.api.layers import Group, PixelLayer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_OUT = os.path.join(ROOT, "tmp", "psd-fixtures")


def solid(size, color):
    return Image.new("RGBA", size, color)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    out = os.path.join(args.out, "nested-groups.psd")

    canvas = (64, 48)
    psd = PSDImage.new(mode="RGB", size=canvas)

    def layer(parent, name, color, left=4, top=4, size=(24, 16)):
        return PixelLayer.frompil(
            solid(size, color), parent, name, top=top, left=left
        )

    psd.append(layer(psd, "Background", (20, 24, 30, 255), 0, 0, canvas))

    # 组「按钮」（展开态）
    btn = Group.new(psd, name="Button", open_folder=True)
    btn.append(layer(btn, "Btn Text", (240, 240, 240, 255), 8, 8, (16, 10)))
    btn.append(layer(btn, "Btn Fill", (60, 120, 200, 255), 6, 6, (28, 18)))
    psd.append(btn)

    # 组「外层」（关闭态）里再套「内层」
    inner = Group.new(psd, name="Inner", open_folder=True)
    inner.append(layer(inner, "Inner B", (200, 80, 80, 255), 12, 12, (12, 12)))
    inner.append(layer(inner, "Inner A", (90, 200, 120, 255), 10, 10, (12, 12)))
    outer = Group.new(psd, name="Outer", open_folder=False)
    outer.append(layer(outer, "Outer Loose", (200, 200, 60, 255), 30, 6, (14, 10)))
    outer.append(inner)
    psd.append(outer)

    psd.save(out)
    print("wrote", out, os.path.getsize(out), "bytes")

    # 回读自检：确认 psd-tools 自己也看到了这棵树
    back = PSDImage.open(out)
    for l in back:
        kind = "GROUP" if l.is_group() else "layer"
        kids = len(list(l)) if l.is_group() else 0
        print("  %-5s %-12s open_folder=%s children=%d"
              % (kind, l.name, getattr(l, "open_folder", None), kids))
    return 0


if __name__ == "__main__":
    sys.exit(main())
