import re
path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")
# view block L469..L711 (1-based)
start, end = 468, 710  # 0-based
assert lines[start].strip() == "Flickable {", repr(lines[start])
assert lines[end].strip() == "}", repr(lines[end])
block = "\n".join(lines[start:end+1])
outside = "\n".join(lines[:start] + lines[end+1:])

print("block L%d..L%d (%d lines)" % (start+1, end+1, end-start+1))
# internal ids
ids = re.findall(r"\bid:\s*(\w+)", block)
print("INTERNAL IDS:", ids)

# for each internal id, count external refs (dotted + bare)
print("\n=== external refs to each internal id ===")
for oid in ids:
    dotted = re.findall(r"(?<![\w.])" + oid + r"\.\w+", outside)
    bare = re.findall(r"(?<![\w.])" + oid + r"(?![\w.])", outside)
    if dotted or bare:
        print("  %s: dotted=%d bare=%d  e.g. %s" % (oid, len(dotted), len(bare), dotted[:4]))

# view. members used externally
print("\n=== view. members (external) ===")
for m in sorted(set(re.findall(r"(?<![\w.])view\.(\w+)", outside))):
    print("  view.%s  x%d" % (m, len(re.findall(r"(?<![\w.])view\." + m, outside))))

# root. refs inside block
print("\n=== root. refs in block ===")
for m in sorted(set(re.findall(r"(?<![\w.])root\.\w+", block))):
    print("  %s  x%d" % (m, len(re.findall(r"(?<![\w.])root\." + m.split(".")[1], block))))

# bridge. refs in block
print("\n=== bridge. refs in block ===")
print("  total", len(re.findall(r"(?<![\w.])bridge\.\w+", block)))

# bare EditorContent fns called in block
outside_fns = set(re.findall(r"^\s*function\s+(\w+)\s*\(", outside, re.M))
print("\n=== bare calls to OUTSIDE fns in block ===")
for fn in sorted(outside_fns):
    c = re.findall(r"(?<![\w.])" + fn + r"(?=\s*\()", block)
    if c:
        print("  %s  x%d" % (fn, len(c)))
print("DONE")