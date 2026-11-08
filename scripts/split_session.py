import re

path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

# Find block boundaries (1-based in comments, 0-based in code)
def find(pat):
    for i, l in enumerate(lines):
        if re.match(pat, l):
            return i
    return None

start = find(r"^struct Session \{")
end_marker = find(r"^impl qobject::Bridge \{")
assert start is not None and end_marker is not None, (start, end_marker)
# block = [start, end_marker-1], trim trailing blanks
end = end_marker - 1
while end > start and lines[end].strip() == "":
    end -= 1
print("block L%d-%d (%d lines)" % (start + 1, end + 1, end - start + 1))

block = lines[start:end + 1]

# ---- transform ----
PUB = "pub(in crate::bridge) "
out = []
state = None  # 'struct' | 'enum' | 'impl' | None
for l in block:
    if re.match(r"^struct \w+ \{", l):
        out.append(PUB + l)
        state = "struct"
        continue
    if re.match(r"^enum \w+ \{", l):
        out.append(PUB + l)
        state = "enum"
        continue
    if re.match(r"^const \w+:", l):
        out.append(PUB + l)
        continue
    if re.match(r"^impl Session \{", l):
        out.append(l)
        state = "impl"
        continue
    # closing brace of a top-level item
    if l == "}":
        state = None
        out.append(l)
        continue
    # field inside struct/enum (4-space indent, `name: type` or `name: type,`)
    if state in ("struct", "enum") and re.match(r"^    \w[\w]*\s*:", l):
        out.append("    " + PUB + l.strip())
        continue
    # method / assoc const inside impl (4-space indent)
    if state == "impl" and re.match(r"^    fn ", l):
        out.append("    " + PUB + l[4:])
        continue
    if state == "impl" and re.match(r"^    const ", l):
        out.append("    " + PUB + l[4:])
        continue
    out.append(l)

content = "// Session 渲染管线核心。所有项标 pub(in crate::bridge)，mod.rs 再导出。\nuse super::*;\n\n" + "\n".join(out) + "\n"
open("crates/ui/src/bridge/session.rs", "w", encoding="utf-8").write(content)
print("wrote session.rs (%d lines)" % len(content.split(chr(10))))

# ---- delete block from mod.rs (inclusive [start, end]) ----
del lines[start:end + 1]
new_mod = "\n".join(lines)
open(path, "w", encoding="utf-8").write(new_mod)
print("mod.rs now %d lines" % len(lines))

# ---- add mod decl + re-export ----
m = open(path, encoding="utf-8").read()
m = m.replace("mod paths;\n", "mod paths;\nmod session;\n", 1)
m = m.replace("pub(in crate::bridge) use paths::*;\n",
              "pub(in crate::bridge) use paths::*;\npub(in crate::bridge) use session::*;\n", 1)
open(path, "w", encoding="utf-8").write(m)
print("added mod session + re-export")
print("DONE")