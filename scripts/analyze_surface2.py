import re, io
p = r"crates\ui\qml\editor\EditorContent.qml"
with io.open(p, encoding="utf-8") as f:
    lines = f.readlines()
N = len(lines)
s, e = 680, 1399
def in_block(l):
    return s <= l <= e

# all function defs in whole file
def brace_end(start_idx):
    depth = 0; started = False
    for j in range(start_idx, N):
        seg = lines[j]; k = seg.find('//')
        if k >= 0: seg = seg[:k]
        for ch in seg:
            if ch == '{': depth += 1; started = True
            elif ch == '}':
                depth -= 1
                if started and depth == 0: return j+1
    return None
fdefs = {}
for i, l in enumerate(lines):
    m = re.match(r'\s*function\s+(\w+)\s*\(', l)
    if m:
        fdefs[m.group(1)] = (i+1, brace_end(i))

bt = "".join(lines[s-1:e])
# bare calls in block (word( not preceded by . or word)
calls = set(re.findall(r'(?<![\w.])([A-Za-z_]\w*)\s*\(', bt))
print("=== bare calls in canvasSurface that ARE EditorContent-level functions ===")
for c in sorted(calls):
    if c in fdefs:
        sl, el = fdefs[c]
        loc = "IN-BLOCK" if in_block(sl) else "EditorContent"
        print("  %s (def L%d-%d, %s)" % (c, sl, el, loc))
print("\n=== EditorContent functions defined INSIDE canvasSurface (would move) ===")
for c,(sl,el) in fdefs.items():
    if in_block(sl):
        print("  %s (L%d-%d)" % (c, sl, el))
# textPopup: is it a Popup? check the def line
print("\n=== textPopup def context ===")
for i in range(1276, 1282):
    print("  L%d: %s" % (i+1, lines[i].rstrip()))
# check panMa (view child, uses canvasSurface coords?) - is panMa inside or outside canvasSurface?
print("\n=== panMa def ===")
for i,l in enumerate(lines):
    if re.search(r'id:\s*panMa\b', l):
        print("  L%d: %s" % (i+1, l.rstrip()))
        # show a few lines
        for j in range(i-1, i+6):
            print("   L%d: %s" % (j+1, lines[j].rstrip()[:70]))
        break