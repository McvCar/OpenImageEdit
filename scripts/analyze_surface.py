import re, io
p = r"crates\ui\qml\editor\EditorContent.qml"
with io.open(p, encoding="utf-8") as f:
    lines = f.readlines()
N = len(lines)
s, e = 680, 1399
def in_block(l):
    return s <= l <= e
inner = {}
for i in range(s-1, e):
    m = re.search(r'id:\s*(\w+)', lines[i])
    if m:
        inner.setdefault(m.group(1), i+1)
print("=== canvasSurface internal ids (%d) ===" % len(inner))
for iid, ln in sorted(inner.items(), key=lambda x: x[1]):
    print("  %s @ L%d" % (iid, ln))
print("\n=== external refs to internal ids ===")
for iid in inner:
    locs = [i+1 for i in range(N) if not in_block(i+1) and re.search(r'(?<![\w.])'+iid+r'\b', lines[i])]
    if locs:
        print("  %s: %s" % (iid, locs))
        for ln in locs[:6]:
            print("     L%d: %s" % (ln, lines[ln-1].rstrip()[:80]))
# bare fns / root. / bridge. in block
bt = "".join(lines[s-1:e])
print("\n=== root. refs ===")
print("  ", sorted(set(re.findall(r'(?<![\w.])root\.(\w+)', bt))))
print("=== bridge. refs ===")
print("  ", sorted(set(re.findall(r'(?<![\w.])bridge\.(\w+)', bt))))
print("=== bare calls (word() no dot, not builtin) ===")
calls = set(re.findall(r'(?<![\w.])([A-Za-z_]\w*)\s*\(', bt))
builtin = {"Math","Qt","parseInt","parseFloat","isNaN","String","Number","Array","Object","JSON","Date","console","if","for","while","return","new","function","Math","Math","Qt","requestPaint","getContext","toLocalCoordinate","mapToItem","mapToGlobal","childrenAt","contains","indexOf","slice","substr","floor","round","min","max","abs","push","pop","join","split","map","filter","length","key","toString","toLowerCase","Math","JSON","JSON","JSON","JSON"}
calls = {w for w in calls if w not in builtin and w not in inner}
print("  ", sorted(calls))