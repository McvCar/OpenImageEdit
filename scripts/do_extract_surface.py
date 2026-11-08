import re

path = "crates/ui/qml/editor/EditorContent.qml"
lines = open(path, encoding="utf-8").read().split("\n")

# ---- locate block ----
start = end = None
for i, l in enumerate(lines):
    if re.search(r"\bid:\s*canvasSurface\b", l):
        start = i - 1
        break
assert start is not None and lines[start].strip() == "Item {", repr(lines[start])
depth = 0
for i in range(start, len(lines)):
    depth += lines[i].count("{") - lines[i].count("}")
    if depth == 0:
        end = i
        break
assert end is not None
block_lines = lines[start:end+1]
print("block L%d..L%d (%d lines)" % (start+1, end+1, len(block_lines)))

# ---- build CanvasSurface.qml ----
def dedent(l):
    if l.startswith("    "):
        return l[4:]
    if l.strip() == "":
        return ""
    return l
body = [dedent(l) for l in block_lines]

idx = None
for i, l in enumerate(body):
    if re.search(r"\bid:\s*canvasSurface\b", l):
        idx = i
        break
assert idx is not None
assert body[idx].startswith("    id: canvasSurface"), body[idx]

props = [
    "    required property var host",
    "    required property var bridge",
    "    required property var view",
    "    required property var previewImage",
    "    // 暴露给 EditorContent：光标/探针/选区/缩放/文字编辑等外部交互逻辑",
    "    // 仍留在 EditorContent，通过 instance.alias 访问这些内部 id。",
    "    readonly property alias selDragMouse: selDragMouse",
    "    readonly property alias markMa: markMa",
    "    readonly property alias markPreview: markPreview",
    "    readonly property alias textPopup: textPopup",
    "    readonly property alias scaleTimer: scaleTimer",
]
body = body[:idx+1] + props + body[idx+1:]
t = "\n".join(body)

# ---- replacements inside block ----
n_root = len(re.findall(r"(?<![\w.])root\.", t))
t = re.sub(r"(?<![\w.])root\.", "host.", t)
print("root. -> host.: %d" % n_root)

fns = ["canvasCursor", "findMark", "selectedMarkBox", "settlePaste",
       "snapDisplay", "toDispScale", "toImageScale", "topLayerAt"]
for fn in fns:
    n = len(re.findall(r"(?<![\w.])" + fn + r"(?=\s*\()", t))
    t = re.sub(r"(?<![\w.])" + fn + r"(?=\s*\()", "host." + fn, t)
    print("  %s: %d" % (fn, n))

header = (
    "import QtQuick\n"
    "import com.imageedit\n"
    "import \"../js/marks.js\" as Marks\n"
    "\n"
)
out = header + t
if not out.endswith("\n"):
    out += "\n"
open("crates/ui/qml/panels/CanvasSurface.qml", "w", encoding="utf-8").write(out)
print("wrote CanvasSurface.qml %d lines" % len(out.split("\n")))

# ---- edit EditorContent.qml ----
e = "\n".join(lines)
block_text = "\n".join(block_lines)
assert block_text in e, "block text not found in file"
instance = (
    "    CanvasSurface {\n"
    "        id: canvasSurface\n"
    "        host: root\n"
    "        bridge: bridge\n"
    "        view: view\n"
    "        previewImage: previewImage\n"
    "    }"
)
e = e.replace(block_text, instance, 1)

for pid in ["selDragMouse", "markMa", "markPreview", "scaleTimer", "textPopup"]:
    pat = r"(?<![\w.])" + pid + r"\."
    n = len(re.findall(pat, e))
    e = re.sub(pat, "canvasSurface." + pid + ".", e)
    print("  %s. : %d" % (pid, n))

n_bare = e.count("textPopup: textPopup")
e = e.replace("textPopup: textPopup", "textPopup: canvasSurface.textPopup")
print("  textPopup bare value: %d" % n_bare)

open(path, "w", encoding="utf-8").write(e)
print("wrote EditorContent.qml %d lines" % len(e.split("\n")))

# ---- brace balance sanity ----
for f in [path, "crates/ui/qml/panels/CanvasSurface.qml"]:
    s = open(f, encoding="utf-8").read()
    s2 = re.sub(r"//.*", "", s)
    s2 = re.sub(r'"[^"\n]*"', '""', s2)
    s2 = re.sub(r"'[^'\n]*'", "''", s2)
    print("%s: braces %d/%d" % (f, s2.count("{"), s2.count("}")))
print("DONE")