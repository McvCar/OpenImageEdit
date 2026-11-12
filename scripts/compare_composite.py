"""Compare our PSD layer composite against Photoshop's merged composite.

Reports mean absolute error overall and per horizontal band, so a wrong
background / buttons / title area shows up as a number instead of an opinion.
Also writes a side-by-side PNG.

    python compare_composite.py <ours.png> <psd> [out.png]
"""

import sys

import numpy as np
from PIL import Image
from psd_tools import PSDImage


def main(ours_path, psd_path, out_path="C:/_APP_CODE/compare_ours_vs_ps.png"):
    ours = Image.open(ours_path).convert("RGB")
    psd = PSDImage.open(psd_path)
    ref = psd.topil()
    if ref is None:
        print("no merged composite in this file")
        return
    ref = ref.convert("RGB")
    if ref.size != ours.size:
        ref = ref.resize(ours.size, Image.LANCZOS)

    a = np.asarray(ours).astype(np.float32)
    b = np.asarray(ref).astype(np.float32)
    diff = np.abs(a - b).mean(axis=2)
    print(f"size {ours.size}  MAE {diff.mean():.2f}  P95 {np.percentile(diff, 95):.1f}")
    h = diff.shape[0]
    bands = 8
    for i in range(bands):
        y0, y1 = h * i // bands, h * (i + 1) // bands
        print(f"  rows {y0:>4}-{y1:>4}  MAE {diff[y0:y1].mean():6.2f}")

    # Amplified difference map next to the two renders.
    heat = np.clip(diff * 4, 0, 255).astype(np.uint8)
    heat = np.stack([heat, np.zeros_like(heat), np.zeros_like(heat)], axis=2)
    strip = Image.new("RGB", (ours.width * 3, ours.height), "black")
    strip.paste(ours, (0, 0))
    strip.paste(ref, (ours.width, 0))
    strip.paste(Image.fromarray(heat), (ours.width * 2, 0))
    strip.save(out_path)
    print("wrote", out_path)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], *(sys.argv[3:4]))
