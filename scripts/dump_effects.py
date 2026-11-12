"""Dump raw lfx2 descriptors as plain python structures (for renderer design)."""

import sys
import pprint

from psd_tools import PSDImage
from psd_tools.api.effects import Effects


def plain(v, depth=0):
    if depth > 8:
        return "<deep>"
    from psd_tools.psd.descriptor import Descriptor, List as DList
    from psd_tools.psd.base import BaseElement

    if hasattr(v, "enum") and hasattr(v, "typeID"):
        return f"{v.typeID.decode(errors='replace')}/{v.enum.decode(errors='replace')}"
    if hasattr(v, "unit") and hasattr(v, "value"):
        return f"{v.value}{v.unit.decode(errors='replace') if isinstance(v.unit, bytes) else v.unit}"
    if hasattr(v, "type") and hasattr(v, "value") and not isinstance(v, dict):
        return f"{v.type.decode() if isinstance(v.type, bytes) else v.type}={v.value}"
    if isinstance(v, Descriptor):
        return {plain(k, depth + 1): plain(x, depth + 1) for k, x in v.items()}
    if isinstance(v, DList):
        return [plain(x, depth + 1) for x in v]
    if isinstance(v, BaseElement):
        if hasattr(v, "type") and hasattr(v, "value"):
            # Enumerated / UnitFloat: keep both halves, they matter.
            unit = getattr(v, "unit", None)
            return (v.type, v.value, unit) if unit is not None else (v.type, v.value)
        return v.value if hasattr(v, "value") else str(v)
    if isinstance(v, dict):
        return {plain(k, depth + 1): plain(x, depth + 1) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [plain(x, depth + 1) for x in v]
    return v


def main(path, names):
    psd = PSDImage.open(path)
    seen = set()
    for layer in psd.descendants():
        if names and layer.name not in names:
            continue
        eff = getattr(layer, "effects", None)
        if not eff:
            continue
        for e in eff:
            key = type(e).__name__
            if names and key in seen:
                continue
            seen.add(key)
            print(f"##### {layer.name} / {key}")
            pprint.pprint(plain(e.descriptor), width=140, sort_dicts=False)
            print()


if __name__ == "__main__":
    main(sys.argv[1], set(sys.argv[2:]))
