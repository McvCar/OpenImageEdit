"""Census of layer effects (lfx2) in a real PSD.

Prints which effect types are present / enabled, how many layers carry each,
and a few full parameter dumps so the renderer can be written against real
data instead of guesses.

    python census_effects.py <file.psd>
"""

import collections
import sys

from psd_tools import PSDImage
from psd_tools.api.effects import (
    ColorOverlay,
    DropShadow,
    GradientOverlay,
    InnerGlow,
    InnerShadow,
    OuterGlow,
    PatternOverlay,
    Stroke,
)

NAMES = {
    ColorOverlay: "colorOverlay(SoFi)",
    GradientOverlay: "gradientOverlay(Grdn)",
    Stroke: "stroke(FrFX)",
    InnerShadow: "innerShadow(IrSh)",
    DropShadow: "dropShadow(DrSh)",
    OuterGlow: "outerGlow(OrGl)",
    InnerGlow: "innerGlow(IrGl)",
    PatternOverlay: "patternOverlay(PtFl)",
}


def main(path):
    psd = PSDImage.open(path)
    total = 0
    with_effects = 0
    enabled_counter = collections.Counter()
    present_counter = collections.Counter()
    samples = collections.defaultdict(list)

    for layer in psd.descendants():
        total += 1
        effects = getattr(layer, "effects", None)
        if not effects or len(effects) == 0:
            continue
        with_effects += 1
        for eff in effects:
            key = NAMES.get(type(eff), type(eff).__name__)
            present_counter[key] += 1
            enabled = getattr(eff, "enabled", None)
            if enabled:
                enabled_counter[key] += 1
            if len(samples[key]) < 4:
                samples[key].append((layer.name, repr(eff)))

    print(f"layers {total}  with_effects {with_effects}")
    for key, n in present_counter.most_common():
        print(f"  {key:<26} present {n:>4}  enabled {enabled_counter[key]:>4}")
    for key, vals in samples.items():
        print(f"\n=== {key} samples")
        for name, text in vals:
            print(f"  [{name}] {text[:900]}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd")
