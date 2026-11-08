import re
path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")
start, end = 679, 1398  # 0-based: L680..L1399
block = "\n".join(lines[start:end+1])
outside = "\n".join(lines[:start] + lines[end+1:])

# all ids defined anywhere
all_ids = set(re.findall(r"\bid:\s*(\w+)", open(path,encoding="utf-8").read()))
# ids defined outside block
outside_ids = set(re.findall(r"\bid:\s*(\w+)", outside))
# ids defined inside block
inside_ids = set(re.findall(r"\bid:\s*(\w+)", block))

# QML/Qt keywords & builtins to ignore
kw = set("""item rectangle image canvas text mousearea timer flickable loader row column
anchors property var readonly function return if else const let var true false null undefined
qt console math point position mouse event width height x y z visible enabled anchors
parent root bridge view panMa previewImage imageedit true false number string int real bool
parent window screen application qml qt console math json date regexp url color size point
pointer hovered accepted acceptedButtons pressed cursorShape pointChanged hoveredChanged
parent parent parent""".split())

print("=== OUTSIDE ids referenced BARE (no dot) in block ===")
for oid in sorted(outside_ids):
    if oid in inside_ids:
        continue
    # bare token, not preceded/followed by word char or dot
    m = re.findall(r"(?<![\w.])" + oid + r"(?![\w.])", block)
    if m and oid not in kw:
        print(f"  {oid}: {len(m)}")

print("\n=== also check function-ish names (EditorContent fns) used bare ===")
for fn in sorted(set(re.findall(r"^\s*function\s+(\w+)\s*\(", outside, re.M))):
    m = re.findall(r"(?<![\w.])" + fn + r"(?![\w.])", block)
    if m:
        print(f"  {fn}: {len(m)}")
print("DONE")