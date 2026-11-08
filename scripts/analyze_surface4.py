import re, io
p = r"crates\ui\qml\editor\EditorContent.qml"
with io.open(p, encoding="utf-8") as f:
    lines = f.readlines()
N = len(lines)
s, e = 680, 1399
def in_block(l):
    return s <= l <= e
bt = "".join(lines[s-1:e])

# all id definitions in whole file
id_lines = {}
for i, l in enumerate(lines):
    m = re.search(r'id:\s*(\w+)', l)
    if m:
        id_lines.setdefault(m.group(1), i+1)

# internal ids of the block
inner = {iid for iid, ln in id_lines.items() if in_block(ln)}
print("internal ids:", sorted(inner))

# find all identifiers followed by '.' in the block, that are NOT internal ids,
# NOT builtin, and ARE defined as ids somewhere
builtin = {"root","bridge","b","host","modelData","mouse","parent","window","Qt","Math",
 "point","ctx","canvas","text","font","anchors","border","elide","visible","enabled",
 "width","height","x","y","color","radius","spacing","implicitWidth","implicitHeight",
 "drop","item","requestPaint","context","Text","Item","Rectangle","Row","Column","Image",
 "MouseArea","Timer","Repeater","DropArea","TextEdit","TextInput","GridView","ListView",
 "Flickable","Loader","Popup","Dialog","Menu","MenuItem","Grid","NumberAnimation",
 "ColorAnimation","Layer","SystemPalette","StandardButton","StandardKey","DialogButtonBox",
 "c","ink","getContext","get","set","find","length","indexOf","push","pop","join","split",
 "map","filter","slice","substr","toLocalCoordinate","mapToItem","mapToGlobal","positionAt",
 "childrenAt","contains","containsMouse","children","objectName","signal","required",
 "property","readonly","function","import","new","null","true","false","undefined","this",
 "Date","JSON","Array","Object","String","Number","Boolean","console","isNaN","parseInt",
 "parseFloat","Math","NaN","Infinity","toString","toFixed","floor","round","min","max","abs",
 "Keys","Qt","MouseArea","HoverHandler","Timer","Repeater","Rectangle","Text","Item","Image",
 "Canvas","MouseArea","DashedRect","EdgeHandles","EdgeGrab","IconButton","ToolButton"}
dotrefs = set(re.findall(r'(?<![\w.])([A-Za-z_]\w*)\.', bt))
outside_ids = {w for w in dotrefs if w in id_lines and w not in inner and w not in builtin}
print("\n=== block 引用的块外 id (word. 形式) ===")
for w in sorted(outside_ids):
    ln = id_lines[w]
    owner = "IN-BLOCK" if in_block(ln) else "outside L%d" % ln
    # count
    c = len(re.findall(r'(?<![\w.])'+w+r'\.', bt))
    print("  %s  (%s)  x%d" % (w, owner, c))

# bare function calls that are EditorContent-level (defined outside block)
def brace_end(start_idx):
    depth=0; started=False
    for j in range(start_idx, N):
        seg=lines[j]; k=seg.find('//')
        if k>=0: seg=seg[:k]
        for ch in seg:
            if ch=='{': depth+=1; started=True
            elif ch=='}':
                depth-=1
                if started and depth==0: return j+1
    return None
fdefs={}
for i,l in enumerate(lines):
    m=re.match(r'\s*function\s+(\w+)\s*\(', l)
    if m: fdefs[m.group(1)]=(i+1, brace_end(i))
calls=set(re.findall(r'(?<![\w.])([A-Za-z_]\w*)\s*\(', bt))
print("\n=== block 裸调用且定义在块外(需 host.) ===")
for c in sorted(calls):
    if c in fdefs:
        sl,el=fdefs[c]
        if not in_block(sl):
            print("  %s (def L%d)" % (c, sl))