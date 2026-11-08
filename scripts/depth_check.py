lines = open("crates/ui/qml/editor/EditorContent.qml", encoding="utf-8").read().split("\n")
depth = 0
for i in range(468, 712):
    l = lines[i]
    depth += l.count("{") - l.count("}")
    if i + 1 in (680, 688, 696, 704, 711):
        print("L%d depth=%d %s" % (i + 1, depth, l.strip()[:30]))