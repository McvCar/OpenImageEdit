import re, sys

path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")

# Block = canvasSurface Item: find `id: canvasSurface` then its opening line
start = None
for i, l in enumerate(lines):
    if re.search(r"\bid:\s*canvasSurface\b", l):
        start = i - 1  # the `Item {` line
        break
assert start is not None, "canvasSurface not found"
# Block end: match braces from start
depth = 0
end = None
for i in range(start, len(lines)):
    depth += lines[i].count("{") - lines[i].count("}")
    if depth == 0:
        end = i
        break
assert end is not None, "block end not found"
print(f"BLOCK: L{start+1}..L{end+1}  ({end-start+1} lines)")
print(f"first: {lines[start].strip()!r}")
print(f"last:  {lines[end].strip()!r}")

block = "\n".join(lines[start:end+1])

# --- internal ids defined in block ---
ids = re.findall(r"\bid:\s*(\w+)", block)
print("\nINTERNAL IDS:", ids)

# --- refs to block-external ids (word. form) outside block ---
outside = "\n".join(lines[:start] + lines[end+1:])
# ids that exist outside and are referenced in block as `id.`
outside_ids = set(re.findall(r"\bid:\s*(\w+)", outside))
print("\n--- block refs to OUTSIDE ids (word.) ---")
for oid in sorted(outside_ids):
    # word-boundary ref: oid. but not preceded by [A-Za-z0-9_.]
    refs = re.findall(r"(?<![\w.])" + oid + r"\.\w+", block)
    if refs:
        print(f"  {oid}: {len(refs)}  e.g. {refs[:4]}")

# --- root. refs ---
rootrefs = re.findall(r"(?<![\w.])root\.\w+", block)
print(f"\nroot. refs: {len(rootrefs)}")
for r in sorted(set(rootrefs)):
    print(f"  {r}  x{rootrefs.count(r)}")

# --- bridge. refs ---
bridg = re.findall(r"(?<![\w.])bridge\.\w+", block)
print(f"\nbridge. refs: {len(bridg)}")
for r in sorted(set(bridg)):
    print(f"  {r}  x{bridg.count(r)}")

# --- bare function calls defined outside ---
# EditorContent-level functions: functions defined in outside at top level
# (function name( ... at 0-indent-ish). Collect function names outside.
outside_fns = set(re.findall(r"^\s*function\s+(\w+)\s*\(", outside, re.M))
print("\n--- bare calls to OUTSIDE functions ---")
for fn in sorted(outside_fns):
    # call site: fn( not preceded by [A-Za-z0-9_.] and not `function fn`
    calls = re.findall(r"(?<![\w.])" + fn + r"\s*\(", block)
    if calls:
        print(f"  {fn}  x{len(calls)}")

# --- root. fns that are actually EditorContent functions (redundant) ---
print("\nDONE")