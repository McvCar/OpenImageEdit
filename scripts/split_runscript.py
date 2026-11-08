import re

path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

# locate run_script
s = None
for i, l in enumerate(lines):
    if l.strip() == "fn run_script(mut self: Pin<&mut Self>, script: &str) {":
        s = i
        break
assert s is not None
d = 0
e = None
for j in range(s, len(lines)):
    d += lines[j].count("{") - lines[j].count("}")
    if d == 0 and j > s:
        e = j
        break
print("run_script L%d..L%d (%d lines)" % (s+1, e+1, e-s+1))

body = "\n".join(lines[s:e+1])
# make it visible to sibling ops modules (shell.rs calls it)
body = body.replace("fn run_script(", "pub(in crate::bridge) fn run_script(", 1)

header = (
    "//! 探针脚本引擎：`--script=op;op;...` 自动化路径。\n"
    "use super::super::*;\n"
    "\n"
)
content = header + "impl qobject::Bridge {\n" + body + "\n}\n"
open("crates/ui/src/bridge/ops/script.rs", "w", encoding="utf-8").write(content)
print("wrote ops/script.rs")

# remove from mod.rs (drop method + trailing blank)
end = e
while end < len(lines) and lines[end].strip() == "":
    end += 1
del lines[s:end]
open(path, "w", encoding="utf-8").write("\n".join(lines))
print("mod.rs now %d lines" % len(lines))

# register in ops/mod.rs
om = open("crates/ui/src/bridge/ops/mod.rs", encoding="utf-8").read()
if "mod script;" not in om:
    om = om.replace("pub(super) mod shell;", "pub(super) mod script;\npub(super) mod shell;")
    open("crates/ui/src/bridge/ops/mod.rs", "w", encoding="utf-8").write(om)
    print("registered script in ops/mod.rs")
print("DONE")