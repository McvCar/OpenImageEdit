"""Extract a balanced QML line range into a new file, with balance checks.

Usage:
    python qml_extract.py <src.qml> <start> <end> <target> [--header HEADER] [--strip-ids] [--dedent]

Checks that the 1-based inclusive range [start, end] is a set of complete
brace blocks at the enclosing container's depth, lists the `id:` declarations
inside (so they can be re-added at the instance site), and writes the block to
<target>. The source file is NOT modified — remove the block there with the
edit tool afterwards.

Options:
    --header HEADER   text to prepend (e.g. the import lines); may contain \n
    --strip-ids       drop `id: X` lines from the extracted block
    --dedent          strip the common leading indentation
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from qml_depth import scan  # noqa: E402


def main():
    args = sys.argv[1:]
    opts = {"header": "", "strip_ids": False, "dedent": False}
    pos = []
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--header":
            opts["header"] = args[i + 1]
            i += 2
            continue
        if a == "--strip-ids":
            opts["strip_ids"] = True
            i += 1
            continue
        if a == "--dedent":
            opts["dedent"] = True
            i += 1
            continue
        pos.append(a)
        i += 1
    if len(pos) != 4:
        print(__doc__)
        sys.exit(1)
    src_path, a, b, target = pos[0], int(pos[1]), int(pos[2]), pos[3]
    lines, depths = scan(src_path)
    if not (1 <= a <= b <= len(lines)):
        print(f"range out of bounds: {a}..{b} (file has {len(lines)} lines)")
        sys.exit(1)
    L = depths[a - 1]
    if b >= len(lines):
        print("refusing: range touches the last line (no depth after it)")
        sys.exit(1)
    end_depth = depths[b]
    if end_depth != L:
        print(f"UNBALANCED: start depth {L}, end depth {end_depth} - check boundaries")
        sys.exit(1)
    bad = [i + 1 for i in range(a - 1, b) if depths[i] < L]
    if bad:
        print(f"UNBALANCED: lines drop below container depth: {bad[:5]}")
        sys.exit(1)
    first_opens = depths[a - 1] == L and depths[a] > L
    if not first_opens:
        print("WARNING: first line does not open a brace block")

    block = lines[a - 1 : b]
    ids = []
    for l in block:
        m = re.match(r"\s*id:\s*([A-Za-z_]\w*)", l)
        if m:
            ids.append(m.group(1))
    print(f"OK: {a}..{b} ({b - a + 1} lines), container depth {L}")
    print(f"ids inside: {', '.join(ids) if ids else '- none -'}")

    out = list(block)
    if opts["strip_ids"]:
        out = [l for l in out if not re.match(r"\s*id:\s*[A-Za-z_]\w*", l)]
    if opts["dedent"]:
        indents = [len(l) - len(l.lstrip()) for l in out if l.strip()]
        pad = min(indents) if indents else 0
        out = [l[pad:] if l.strip() else "" for l in out]
    header = opts["header"].splitlines() if opts["header"] else []
    with open(target, "w", encoding="utf-8", newline="\n") as f:
        for h in header:
            f.write(h + "\n")
        if header and out:
            f.write("\n")
        for l in out:
            f.write(l + "\n")
    print(f"wrote {target} ({len(header) + len(out)} lines)")


if __name__ == "__main__":
    main()