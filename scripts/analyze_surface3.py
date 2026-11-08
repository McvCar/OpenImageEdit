import re, io
p = r"crates\ui\qml\editor\EditorContent.qml"
with io.open(p, encoding="utf-8") as f:
    lines = f.readlines()
N = len(lines)
s, e = 680, 1399
def in_block(l):
    return s <= l <= e
# external code refs (non-comment) to the 5 ids
ids = ["selDragMouse", "markMa", "markPreview", "textPopup", "scaleTimer"]
for iid in ids:
    print("=== %s external refs ===" % iid)
    for i in range(N):
        if in_block(i+1):
            continue
        l = lines[i]
        if re.search(r'(?<![\w.])'+iid+r'\b', l):
            # skip pure comment lines
            stripped = l.strip()
            is_comment = stripped.startswith('//')
            tag = "(comment)" if is_comment else ""
            print("  L%d %s: %s" % (i+1, tag, stripped[:90]))
PYEOF