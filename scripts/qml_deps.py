"""Report external-id references and bare host-function calls for QML line ranges.

An extracted file can only see what it is given, so every external id that a
region touches has to become an inbound property, and every unqualified call to
the enclosing component's own function must be qualified before the move.
This makes both lists exact instead of guessed.

Usage:
    python qml_deps.py <file.qml> <start> <end> [start end ...]

For each 1-based inclusive range [start, end]:
  * external ids: identifiers declared somewhere in the file via `id: X`
    but not declared inside the range;
  * bare calls: calls to functions declared at the same brace depth as the
    range's first line (i.e. the enclosing component's own functions) that
    appear unqualified inside the range.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qml_depth import scan  # noqa: E402


def strip_code(lines):
    out = []
    for l in lines:
        l = re.sub(r"//.*$", "", l)
        l = re.sub(r'"(?:[^"\\]|\\.)*"', '""', l)
        l = re.sub(r"'(?:[^'\\]|\\.)*'", "''", l)
        out.append(l)
    return out


def main():
    if len(sys.argv) < 4 or (len(sys.argv) - 2) % 2:
        print(__doc__)
        sys.exit(1)
    path = sys.argv[1]
    rest = sys.argv[2:]
    ranges = [(int(rest[i]), int(rest[i + 1])) for i in range(0, len(rest), 2)]
    lines, depths = scan(path)
    nocomment = strip_code(lines)

    decl = {}
    for i, l in enumerate(lines, 1):
        m = re.match(r"\s*id:\s*([A-Za-z_]\w*)", l)
        if m:
            decl.setdefault(m.group(1), i)

    for a, b in ranges:
        body = "\n".join(nocomment[a - 1 : b])
        L = depths[a - 1]
        used = {}
        for m in re.finditer(r"\b([A-Za-z_]\w*)\s*\.", body):
            ident = m.group(1)
            if ident in decl and not (a <= decl[ident] <= b):
                used[ident] = used.get(ident, 0) + 1
        host_funcs = set()
        for i in range(len(lines)):
            if depths[i] == L:
                m = re.match(r"\s*function\s+([A-Za-z_]\w*)\s*\(", lines[i])
                if m:
                    host_funcs.add(m.group(1))
        bare = {}
        if host_funcs:
            pat = r"(?<![\w.])(" + "|".join(map(re.escape, sorted(host_funcs))) + r")\s*\("
            for m in re.finditer(pat, body):
                fn = m.group(1)
                bare[fn] = bare.get(fn, 0) + 1
        print(f"== {a}..{b} ({b - a + 1} lines) ==")
        txt = ", ".join(f"{k}({v})" for k, v in sorted(used.items(), key=lambda kv: -kv[1]))
        print(f"  external ids: {txt if txt else '- none -'}")
        txt = ", ".join(f"{k}({v})" for k, v in sorted(bare.items(), key=lambda kv: -kv[1]))
        print(f"  bare calls:   {txt if txt else '- none -'}")


if __name__ == "__main__":
    main()