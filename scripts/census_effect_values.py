"""Value census for layer effects, so the renderer implements the variants that
actually occur. Prints counts of enums and the max numeric parameters."""

import collections
import sys

from psd_tools import PSDImage
from psd_tools.api.effects import (
    ColorOverlay,
    DropShadow,
    GradientOverlay,
    InnerGlow,
    InnerShadow,
    Stroke,
)


def enum(v):
    if v is None:
        return "none"
    return getattr(v, "value", v)


def num(v):
    try:
        return float(getattr(v, "value", v))
    except Exception:
        return None


def main(path):
    psd = PSDImage.open(path)
    c = collections.Counter()
    mx = collections.defaultdict(float)
    grad_types = collections.Counter()
    stop_counts = collections.Counter()

    for layer in psd.descendants():
        eff = getattr(layer, "effects", None)
        if not eff:
            continue
        for e in eff:
            if not getattr(e, "enabled", False):
                continue
            if isinstance(e, Stroke):
                c[f"stroke pos={enum(e.position)} fill={enum(e.fill_type)}"] += 1
                mx["stroke size"] = max(mx["stroke size"], num(e.size) or 0)
            elif isinstance(e, DropShadow):
                c[f"drop blend={enum(e.blend_mode)} knock={e.layer_knocks_out}"] += 1
                for k, v in (("drop size", e.size), ("drop dist", e.distance), ("drop choke", e.choke)):
                    mx[k] = max(mx[k], num(v) or 0)
            elif isinstance(e, InnerShadow):
                c[f"inner blend={enum(e.blend_mode)}"] += 1
                for k, v in (("inner size", e.size), ("inner dist", e.distance), ("inner choke", e.choke)):
                    mx[k] = max(mx[k], num(v) or 0)
            elif isinstance(e, GradientOverlay):
                c[f"grad blend={enum(e.blend_mode)} type={enum(e.type)} rev={e.reversed} algn={e.aligned}"] += 1
                g = e.gradient
                stops = list(g) if g is not None else []
                stop_counts[len(stops)] += 1
                grad_types["stops"] += 1
                mx["grad scale"] = max(mx["grad scale"], num(e.scale) or 0)
            elif isinstance(e, ColorOverlay):
                c[f"coloroverlay blend={enum(e.blend_mode)}"] += 1
            elif isinstance(e, InnerGlow):
                c[f"innerglow blend={enum(e.blend_mode)} src={enum(e.glow_source)}"] += 1
                mx["innerglow size"] = max(mx["innerglow size"], num(e.size) or 0)
    for k, v in sorted(c.items()):
        print(f"{v:>5}  {k}")
    print()
    for k, v in sorted(mx.items()):
        print(f"max {k:<14} {v:.1f}")
    print("gradient stop counts:", dict(stop_counts))
    print("gradient types:", dict(grad_types))


if __name__ == "__main__":
    main(sys.argv[1])
