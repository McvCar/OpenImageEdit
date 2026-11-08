import re
path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")
start, end = 468, 710  # 0-based L469..L711
block = "\n".join(lines[start:end+1])
outside = "\n".join(lines[:start] + lines[end+1:])

# external refs to child ids (need alias + prefix)
print("=== external refs to child ids (dotted / bare) ===")
for oid in ["previewImage", "canvasSurface", "selectionViz", "rulerOverlay"]:
    dotted = re.findall(r"(?<![\w.])" + oid + r"\.\w+", outside)
    bare = re.findall(r"(?<![\w.])" + oid + r"(?![\w.])", outside)
    print("  %s: dotted=%d bare=%d" % (oid, len(dotted), len(bare)))
    # show bare contexts
    for m in re.finditer(r"(?<![\w.])" + oid + r"(?![\w.])", outside):
        s = max(0, m.start()-25)
        ctx = outside[s:m.end()+5].replace("\n", "\\n")
        print("      bare ctx: ...%s" % ctx)

# external-id refs INSIDE block (anchors etc.)
print("\n=== external-id refs inside block ===")
for oid in ["markBar", "toolbar", "statusBar", "layerPanel", "adjustPanel",
            "imageRendered", "panMa", "root", "bridge"]:
    refs = re.findall(r"(?<![\w.])" + oid + r"(?![\w.])", block)
    dots = re.findall(r"(?<![\w.])" + oid + r"\.\w+", block)
    if refs or dots:
        print("  %s: bare=%d dotted=%d" % (oid, len(refs), len(dots)))

# imageRendered def location
print("\n=== imageRendered def ===")
for i, l in enumerate(lines):
    if re.search(r"property\s+\w+\s+imageRendered\b", l):
        print("  L%d: %s" % (i+1, l.strip()))
print("DONE")