import re

path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

def find_mod(name):
    # `#[cfg(test)]` then `mod name {`
    for i, l in enumerate(lines):
        if re.match(r"^mod %s \{" % re.escape(name), l):
            s = i
            if s > 0 and lines[s-1].strip() == "#[cfg(test)]":
                s -= 1
            d = 0; e = None; started = False
            for j in range(i, len(lines)):
                d += lines[j].count("{") - lines[j].count("}")
                if "{" in lines[j]: started = True
                if started and d == 0:
                    e = j; break
            return s, e
    return None

mods = {
    "preview_target_tests": "geom",
    "selection_after_removal_tests": "geom",
    "psd_save_guard_tests": "paths",
}

blocks = {}
for name, dest in mods.items():
    se = find_mod(name)
    assert se, "not found %s" % name
    s, e = se
    blocks[name] = (s, e, dest)
    print("  L%d-%d %s -> %s.rs" % (s+1, e+1, name, dest))

# delete from mod.rs (descending)
for name, (s, e, dest) in sorted(blocks.items(), key=lambda kv: kv[1][0], reverse=True):
    end = e + 1
    while end < len(lines) and lines[end].strip() == "":
        end += 1
    del lines[s:end]
open(path, "w", encoding="utf-8").write("\n".join(lines))
print("mod.rs now %d lines" % len(lines))

# append to dest files
dests = {}
for name, (s, e, dest) in blocks.items():
    dests.setdefault(dest, []).append(name)
# re-read original block text from git HEAD (mod.rs reverted? no, current). Capture before delete is done.
# We already deleted; capture from the in-memory copy we had. Re-extract from a fresh git show of the committed state?
# Simpler: we saved nothing; re-read blocks from the pre-delete content via git.
import subprocess
orig = subprocess.check_output(["git","show","HEAD:crates/ui/src/bridge/mod.rs"]).decode("utf-8").split("\n")
def find_mod_o(name):
    for i, l in enumerate(orig):
        if re.match(r"^mod %s \{" % re.escape(name), l):
            s = i
            if s > 0 and orig[s-1].strip() == "#[cfg(test)]":
                s -= 1
            d=0;e=None;st=False
            for j in range(i,len(orig)):
                d+=orig[j].count("{")-orig[j].count("}")
                if "{" in orig[j]: st=True
                if st and d==0: e=j; break
            return s,e
    return None

for dest, names in dests.items():
    parts = []
    for name in names:
        s, e = find_mod_o(name)
        block = "\n".join(orig[s:e+1])
        # `super` would refer to geom/paths after the move; point at bridge.
        block = block.replace("use super::*;", "use crate::bridge::*;")
        block = block.replace("use super::selection_after_removal;", "use crate::bridge::selection_after_removal;")
        block = block.replace("use super::file_url;", "use crate::bridge::file_url;")
        parts.append(block)
    text = "\n\n".join(parts) + "\n"
    with open("crates/ui/src/bridge/%s.rs" % dest, "a", encoding="utf-8") as f:
        f.write("\n" + text)
    print("appended %s to %s.rs" % (", ".join(names), dest))
print("DONE")