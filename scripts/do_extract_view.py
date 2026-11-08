import re

path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")

# ---- locate view Flickable block: L469..L711 (1-based) ----
start, end = 468, 710  # 0-based
assert lines[start].strip() == "Flickable {", repr(lines[start])
assert lines[end].strip() == "}", repr(lines[end])
block_lines = lines[start:end+1]
print("block L%d..L%d (%d lines)" % (start+1, end+1, len(block_lines)))

# The block is already at component-root indentation (Flickable at col 0),
# so no dedent needed. Find `id: view` line to insert props/aliases after it.
idx = None
for i, l in enumerate(block_lines):
    if re.search(r"\bid:\s*view\b", l):
        idx = i
        break
assert idx is not None, "id: view not found in block"
assert block_lines[idx].strip() == "id: view", block_lines[idx]

props = [
    "    required property var host",
    "    required property var bridge",
    "    required property var markBar",
    "    required property var toolbar",
    "    required property var statusBar",
    "    required property var layerPanel",
    "    // 暴露给 EditorContent：根级缩放/选区函数需读 previewImage 几何；",
    "    // 光标/探针/选区逻辑访问 canvasSurface 与 selectionViz 内部 id。",
    "    readonly property alias previewImage: previewImage",
    "    readonly property alias canvasSurface: canvasSurface",
    "    readonly property alias selectionViz: selectionViz",
]
body = block_lines[:idx+1] + props + block_lines[idx+1:]
t = "\n".join(body)

# ---- replacements inside block ----
# 1) panel instances: host: root -> host: host
n = t.count("host: root")
t = t.replace("host: root", "host: host")
print("host: root -> host: host: %d" % n)
# 2) root.remapSelectionToScale -> host.remapSelectionToScale
n = len(re.findall(r"(?<![\w.])root\.remapSelectionToScale", t))
t = re.sub(r"(?<![\w.])root\.remapSelectionToScale", "host.remapSelectionToScale", t)
print("root.remapSelectionToScale -> host.: %d" % n)
# 3) imageRendered = true -> host.imageRendered = true
n = len(re.findall(r"(?<![\w.])imageRendered\s*=", t))
t = re.sub(r"(?<![\w.])imageRendered\s*=", "host.imageRendered =", t)
print("imageRendered = -> host.imageRendered =: %d" % n)

header = "import QtQuick\nimport com.imageedit\n\n"
out = header + t
if not out.endswith("\n"):
    out += "\n"
open("crates/ui/qml/panels/CanvasView.qml", "w", encoding="utf-8").write(out)
print("wrote CanvasView.qml %d lines" % len(out.split("\n")))

# ---- edit EditorContent.qml ----
e = "\n".join(lines)
block_text = "\n".join(block_lines)
assert block_text in e, "block text not found"
instance = (
    "CanvasView {\n"
    "    id: view\n"
    "    host: root\n"
    "    bridge: bridge\n"
    "    markBar: markBar\n"
    "    toolbar: toolbar\n"
    "    statusBar: statusBar\n"
    "    layerPanel: layerPanel\n"
    "}"
)
e = e.replace(block_text, instance, 1)

# ---- prefix external refs to child ids with view. ----
for oid in ["previewImage", "canvasSurface", "selectionViz"]:
    pat = r"(?<![\w.])" + oid + r"\."
    n = len(re.findall(pat, e))
    e = re.sub(pat, "view." + oid + ".", e)
    print("  %s. -> view.%s. : %d" % (oid, oid, n))

open(path, "w", encoding="utf-8").write(e)
print("wrote EditorContent.qml %d lines" % len(e.split("\n")))

# ---- brace balance ----
for f in [path, "crates/ui/qml/panels/CanvasView.qml"]:
    s = open(f, encoding="utf-8").read()
    s2 = re.sub(r"//.*", "", s)
    s2 = re.sub(r'"[^"\n]*"', '""', s2)
    s2 = re.sub(r"'[^'\n]*'", "''", s2)
    print("%s: braces %d/%d" % (f, s2.count("{"), s2.count("}")))
print("DONE")