#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate PSD fixtures for the layered-editing (B plan) work, M0.

Pure standard library: writes a minimal but spec-shaped 8-bit RGB PSD by
hand (RAW planar channels, no RLE), so no Photoshop / PIL is needed. The
byte layout deliberately matches the vendored `psd` parser:

  * layer records are stored top -> bottom (the parser reverses them);
  * bottom/right in the file are exclusive (parser subtracts one);
  * `channel_length` includes the 2-byte compression marker;
  * flags follow the Adobe spec: bit 3 = Photoshop 5+, bit 1 SET = HIDDEN;
  * `8BIMluni` carries the UTF-16BE layer name.

Usage:
    python scripts/make_psd_fixture.py [--out DIR] [--only small|large|all]
"""

import argparse
import json
import os
import struct
import sys
import time
from pathlib import Path

U16 = lambda v: struct.pack(">H", v)
I16 = lambda v: struct.pack(">h", v)
U32 = lambda v: struct.pack(">I", v)
I32 = lambda v: struct.pack(">i", v)

CHANNEL_IDS = (0, 1, 2, -1)  # R, G, B, transparency mask


def pad4(blob):
    return blob + b"\0" * ((-len(blob)) % 4)


def pascal_name(name):
    raw = name.encode("ascii", "replace")[:255]
    return pad4(bytes([len(raw)]) + raw)


def unicode_name(name):
    utf16 = name.encode("utf-16-be")
    payload = pad4(U32(len(utf16) // 2) + utf16)
    return b"8BIM" + b"luni" + U32(len(payload)) + payload


def layer_record(name, top, left, w, h, opacity, visible, blend=b"norm", extra=b""):
    chan_len = 2 + w * h
    channels = b"".join(I16(cid) + U32(chan_len) for cid in CHANNEL_IDS)
    # Adobe spec: bit 3 = "Photoshop 5.0 and later", bit 1 SET = HIDDEN.
    flags = 8 if visible else (8 | 2)
    extra_blob = U32(0) + U32(0) + pascal_name(name) + unicode_name(name) + extra
    return (
        I32(top)
        + I32(left)
        + I32(top + h)
        + I32(left + w)
        + U16(len(CHANNEL_IDS))
        + channels
        + b"8BIM"
        + blend
        + bytes([opacity & 0xFF, 0, flags, 0])
        + U32(len(extra_blob))
        + extra_blob
    )


def channel_data(layer, channel):
    """One RAW planar channel: u16 compression + w*h deterministic bytes."""
    w, h = layer["w"], layer["h"]
    seed, alpha = layer.get("seed", 1), layer.get("alpha", 255)
    out = bytearray(U16(0))
    if channel == 3:
        out += bytes([alpha]) * (w * h)
        return bytes(out)
    rows = [
        bytes(((x * (7 + channel * 3) + y * 5 + seed * 31 + channel * 53) & 0xFF) for x in range(w))
        for y in range(16)
    ]
    block = b"".join(rows)
    full, rest = divmod(h, 16)
    out += block * full
    out += b"".join(rows[y] for y in range(rest))
    return bytes(out)


def composite_channel(width, height, channel, alpha=255):
    if channel == 3:
        row = bytes([alpha]) * width
    else:
        row = bytes(((x * 9 + channel * 41) & 0xFF) for x in range(width))
    block = row * 64
    full, rest = divmod(height, 64)
    return block * full + row * rest


def write_psd(path, width, height, layers, composite_alpha=255):
    """`layers` is bottom -> top; each is a dict with name/top/left/w/h/..."""
    t0 = time.time()
    with open(path, "wb") as fp:
        fp.write(b"8BPS" + U16(1) + b"\0" * 6)
        fp.write(U16(4) + U32(height) + U32(width) + U16(8) + U16(3))
        fp.write(U32(0))  # color mode data
        fp.write(U32(0))  # image resources

        # File order is top -> bottom; the parser reverses it into bottom -> top.
        top_first = list(reversed(layers))
        records = b"".join(
            layer_record(
                layer["name"],
                layer["top"],
                layer["left"],
                layer["w"],
                layer["h"],
                layer.get("opacity", 255),
                layer.get("visible", True),
                layer.get("blend", b"norm"),
                layer.get("extra", b""),
            )
            for layer in top_first
        )
        channels = b"".join(
            channel_data(layer, ci) for layer in top_first for ci in range(4)
        )
        # Layer Info: u32 length (excluding itself) | i16 count | records | channel data
        info_len = 2 + len(records) + len(channels)
        layer_info = U32(info_len) + I16(len(layers)) + records + channels
        # Layer & Mask section length covers Layer Info + Global Layer Mask (u32 0).
        fp.write(U32(len(layer_info) + 4) + layer_info + U32(0))

        fp.write(U16(0))  # Image Data: RAW
        for ci in range(4):
            fp.write(composite_channel(width, height, ci, composite_alpha))
    return {
        "path": str(path),
        "width": width,
        "height": height,
        "layers": len(layers),
        "bytes": os.path.getsize(path),
        "gen_ms": round((time.time() - t0) * 1000, 1),
    }


def small_spec():
    return [
        dict(name="背景底色", top=0, left=0, w=260, h=400, seed=1),
        dict(name="普通图层", top=20, left=30, w=120, h=160, seed=2),
        dict(name="半透明 50%", top=60, left=90, w=100, h=140, opacity=128, seed=3, alpha=200),
        dict(name="隐藏图层", top=100, left=10, w=80, h=90, visible=False, seed=4),
        dict(name="负偏移+越界", top=-25, left=-30, w=300, h=120, blend=b"mul ", seed=5),
    ]


def large_spec():
    layers = [dict(name="大图层-底", top=0, left=0, w=2000, h=1500, seed=11)]
    for i in range(9):
        layers.append(
            dict(
                name=("小图层 %02d" % (i + 1)) if i != 4 else "中文层名-小",
                top=120 + i * 210,
                left=180 + i * 330,
                w=400,
                h=300,
                opacity=255 if i % 3 else 128,
                visible=(i % 4 != 0),
                seed=20 + i,
                alpha=255 if i % 2 else 176,
            )
        )
    return layers


def many_spec(count=100):
    """100 small layers for the C6 thumbnail / C7 heavy-stack checks."""
    return [
        dict(
            name=("层 %03d" % i) if i != 42 else "中文层名-042",
            top=(i * 7) % 240,
            left=(i * 11) % 340,
            w=48,
            h=40,
            opacity=255 if i % 5 else 128,
            visible=(i % 9 != 0),
            seed=100 + i,
            alpha=255 if i % 2 else 200,
        )
        for i in range(count)
    ]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None, help="output dir (default tmp/psd-fixtures)")
    ap.add_argument("--only", choices=["small", "large", "many", "oversize", "all"],
                    default="all")
    args = ap.parse_args()

    root = Path(__file__).resolve().parent.parent
    out = Path(args.out) if args.out else root / "tmp" / "psd-fixtures"
    out.mkdir(parents=True, exist_ok=True)
    results = []

    if args.only in ("small", "all"):
        results.append(write_psd(out / "small-layers.psd", 260, 400, small_spec()))
        results.append(
            write_psd(
                out / "single-layer.psd",
                64,
                48,
                [dict(name="单层", top=0, left=0, w=64, h=48, seed=9)],
            )
        )
        src = (out / "small-layers.psd").read_bytes()
        bad = bytearray(src)
        bad[34:38] = b"\x7f\xff\xff\xff"  # Layer & Mask section length at 26+4+4
        (out / "corrupt.psd").write_bytes(bytes(bad))
        results.append(dict(path=str(out / "corrupt.psd"), bytes=len(bad), note="broken section length"))

    if args.only in ("large", "all"):
        results.append(write_psd(out / "large-10layers.psd", 4000, 3000, large_spec()))

    if args.only in ("many", "all"):
        results.append(write_psd(out / "many-100layers.psd", 400, 300, many_spec()))

    if args.only in ("oversize", "all"):
        # Longest edge 9000 > DOWNSAMPLE_EDGE (8192): with downsampling-first
        # enabled this opens as a read-only composite preview (C8).
        results.append(
            write_psd(
                out / "oversize.psd", 9000, 600,
                [dict(name="超大画布", top=0, left=0, w=64, h=64, seed=7)],
            )
        )

    print(json.dumps({"fixtures": results}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
