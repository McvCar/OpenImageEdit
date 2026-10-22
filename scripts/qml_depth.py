"""Compute true brace depth for a QML file, ignoring comments/strings.

Emits the start/end line of every direct child of a chosen container, so the
split boundaries come from the parser's view of nesting rather than from the
(unreliable) indentation.

Usage:
    python qml_depth.py <file.qml> <container_line> [--min N]

container_line is the 1-based line of `X {` or `component X: Y {`.
--min N filters out child blocks shorter than N lines.
"""
import sys


def scan(path):
    src = open(path, encoding="utf-8").read()
    depth = 0
    depths = []  # depth at the START of each line (1-based index via [lineno-1])
    i = 0
    n = len(src)
    depth_at_line_start = 0
    in_line_comment = False
    in_block_comment = False
    in_str = None  # quote char
    in_tmpl = False
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""

        if c == "\n":
            depths.append(depth_at_line_start)
            depth_at_line_start = depth
            in_line_comment = False
            i += 1
            continue

        if in_line_comment:
            i += 1
            continue

        if in_block_comment:
            if c == "*" and nxt == "/":
                in_block_comment = False
                i += 2
                continue
            i += 1
            continue

        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == in_str:
                in_str = None
            i += 1
            continue

        if in_tmpl:
            if c == "\\":
                i += 2
                continue
            if c == "`":
                in_tmpl = False
            i += 1
            continue

        # not in comment/string
        if c == "/" and nxt == "/":
            in_line_comment = True
            i += 2
            continue
        if c == "/" and nxt == "*":
            in_block_comment = True
            i += 2
            continue
        if c == '"' or c == "'":
            in_str = c
            i += 1
            continue
        if c == "`":
            in_tmpl = True
            i += 1
            continue
        if c == "{":
            depth += 1
            i += 1
            continue
        if c == "}":
            depth -= 1
            i += 1
            continue
        i += 1

    depths.append(depth_at_line_start)
    return src.splitlines(), depths


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    path = sys.argv[1]
    target = int(sys.argv[2])  # 1-based line whose children we want
    min_lines = 1
    if "--min" in sys.argv:
        min_lines = int(sys.argv[sys.argv.index("--min") + 1])
    lines, depths = scan(path)
    # depth at the start of `target`; children start at depth+1
    parent_depth = depths[target - 1]
    child_depth = parent_depth + 1

    print(f"# {path}: {len(lines)} lines; container line {target} depth={parent_depth}")
    print(f"# children open at depth {child_depth}\n")

    def depth_after(idx):
        """Brace depth once line idx (0-based) has been fully read."""
        return depths[idx + 1] if idx + 1 < len(depths) else depths[idx]

    i = target  # 0-based index of the line after the container's opening line
    while i < len(lines):
        ln = i + 1
        if depths[i] < child_depth:
            print(f"# container closes at line {ln - 1}")
            break
        opens = depths[i] == child_depth and depth_after(i) > child_depth
        if opens:
            j = i
            while j + 1 < len(lines) and not (depths[j + 1] == child_depth and depth_after(j + 1) <= child_depth):
                j += 1
            span = j + 1 - ln + 1
            if span >= min_lines:
                print(f"{ln:>5} .. {j + 1:>5}  ({span:>4} lines)  {lines[i].strip()[:66]}")
            i = j + 1
            continue
        if lines[i].strip():
            print(f"{ln:>5}  ----  (1 line, no block)  {lines[i].strip()[:66]}")
        i += 1


if __name__ == "__main__":
    main()