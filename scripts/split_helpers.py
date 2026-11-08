import re

path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

def find_item(name, kind="fn"):
    """Find a top-level item `fn name(` or `struct name {` / `const name:` and return (start,end) 0-based inclusive."""
    for i, l in enumerate(lines):
        if kind == "fn" and re.match(r"^fn %s\(" % re.escape(name), l):
            s = i
            # include preceding doc comments
            while s > 0 and (lines[s-1].startswith("///") or lines[s-1].startswith("//!")):
                s -= 1
            d = 0; e = None; started = False
            for j in range(i, len(lines)):
                d += lines[j].count("{") - lines[j].count("}")
                if "{" in lines[j]: started = True
                if started and d == 0:
                    e = j; break
            return s, e
        if kind == "struct" and re.match(r"^struct %s \{" % re.escape(name), l):
            s = i
            while s > 0 and (lines[s-1].startswith("///") or lines[s-1].startswith("//!")):
                s -= 1
            d = 0; e = None; started = False
            for j in range(i, len(lines)):
                d += lines[j].count("{") - lines[j].count("}")
                if "{" in lines[j]: started = True
                if started and d == 0:
                    e = j; break
            return s, e
        if kind == "const" and re.match(r"^const %s:" % re.escape(name), l):
            # single line const
            return i, i
    return None

def find_impl_for(typ):
    """Find `impl ... typ {` (inherent impl, no trait) and return (start,end)."""
    for i, l in enumerate(lines):
        if re.match(r"^impl %s \{" % re.escape(typ), l):
            s = i
            d = 0; e = None; started = False
            for j in range(i, len(lines)):
                d += lines[j].count("{") - lines[j].count("}")
                if "{" in lines[j]: started = True
                if started and d == 0:
                    e = j; break
            return s, e
    return None

# ---- collect items to move per module ----
plan = {
    "geom": [("fn","fit_zoom"),("fn","display_size"),("fn","size_close"),
             ("fn","preview_target"),("fn","selection_after_removal")],
    "config": [("struct","AppConfig"),("impl","AppConfig"),("fn","human_size")],
    "paths": [("fn","clipboard"),("fn","write_checker_tile_bmp"),("fn","file_url"),
              ("fn","extension_for"),("fn","is_psd_path"),("const","PSD_SAVE_HINT"),
              ("fn","save_format_for_source")],
}

ranges = []  # (start,end,name) to delete from mod.rs
extracted = {g: [] for g in plan}
for g, items in plan.items():
    for kind, name in items:
        if kind == "impl":
            se = find_impl_for(name)
        else:
            se = find_item(name, kind)
        assert se, "not found %s %s" % (kind, name)
        s, e = se
        ranges.append((s, e, "%s.%s" % (g, name)))
        extracted[g].append((s, e, name))

# print for verification
for g, items in extracted.items():
    print(g)
    for s, e, n in items:
        print("  L%d-%d %s" % (s+1, e+1, n))

# ---- delete from mod.rs (desc order) ----
for s, e, _ in sorted(ranges, key=lambda x: x[0], reverse=True):
    end = e
    while end < len(lines) and lines[end].strip() == "":
        end += 1
    del lines[s:end]
new_mod = "\n".join(lines)
open(path, "w", encoding="utf-8").write(new_mod)
print("mod.rs now %d lines" % len(lines))

# ---- write each helper module ----
# recompute ranges on original (already deleted, so use saved line content captured BEFORE delete)
# We captured content implicitly; redo extraction from a fresh read is impossible now.
# Instead, capture content BEFORE deletion: re-run on original.
orig = open(path, encoding="utf-8").read()  # this is post-delete; need original
# We'll re-extract from git HEAD version instead.
import subprocess
orig = subprocess.check_output(["git","show","HEAD:crates/ui/src/bridge/mod.rs"]).decode("utf-8")
olines = orig.split("\n")

def find_item_o(name, kind="fn"):
    for i, l in enumerate(olines):
        if kind == "fn" and re.match(r"^fn %s\(" % re.escape(name), l):
            s = i
            while s > 0 and (olines[s-1].startswith("///") or olines[s-1].startswith("//!")):
                s -= 1
            d=0;e=None;st=False
            for j in range(i,len(olines)):
                d+=olines[j].count("{")-olines[j].count("}")
                if "{" in olines[j]: st=True
                if st and d==0: e=j; break
            return s,e
        if kind=="struct" and re.match(r"^struct %s \{" % re.escape(name), l):
            s=i
            while s>0 and (olines[s-1].startswith("///") or olines[s-1].startswith("//!")):
                s-=1
            d=0;e=None;st=False
            for j in range(i,len(olines)):
                d+=olines[j].count("{")-olines[j].count("}")
                if "{" in olines[j]: st=True
                if st and d==0: e=j; break
            return s,e
        if kind=="const" and re.match(r"^const %s:" % re.escape(name), l):
            return i,i
    return None

def find_impl_o(typ):
    for i,l in enumerate(olines):
        if re.match(r"^impl %s \{" % re.escape(typ), l):
            d=0;e=None;st=False
            for j in range(i,len(olines)):
                d+=olines[j].count("{")-olines[j].count("}")
                if "{" in olines[j]: st=True
                if st and d==0: e=j; break
            return i,e
    return None

for g, items in plan.items():
    parts = []
    for kind, name in items:
        se = find_impl_o(name) if kind=="impl" else find_item_o(name, kind)
        s,e = se
        parts.append("\n".join(olines[s:e+1]))
    header = "// %s 相关项。所有项标 pub(in crate::bridge)，mod.rs 再导出。\n" % g
    header += "use super::super::*;\n\n"
    # wrap free fns as-is; they're pub(in crate::bridge)
    body = "\n\n".join(parts)
    # make each `fn name` -> `pub(in crate::bridge) fn name`, `struct X` -> `pub(in crate::bridge) struct X`,
    # `const X` -> `pub(in crate::bridge) const X`, `impl X {` stays, but its methods need pub(in crate::bridge)
    body = re.sub(r"^fn ", "pub(in crate::bridge) fn ", body, flags=re.M)
    body = re.sub(r"^struct ", "pub(in crate::bridge) struct ", body, flags=re.M)
    body = re.sub(r"^const ", "pub(in crate::bridge) const ", body, flags=re.M)
    # fields of pub structs need visibility for cross-module literal construction
    # (AppConfig is constructed by literal in mod.rs). Make field lines pub(in crate::bridge).
    body = re.sub(r"^    (\w[\w:<>]*\s+[\w,]+)$", r"    pub(in crate::bridge) \1", body, flags=re.M)
    # impl methods: `    fn name` -> `    pub(in crate::bridge) fn name`
    body = re.sub(r"^    fn ", "    pub(in crate::bridge) fn ", body, flags=re.M)
    content = header + body + "\n"
    open("crates/ui/src/bridge/%s.rs" % g, "w", encoding="utf-8").write(content)
    print("wrote %s.rs" % g)

# ---- add mod decls + re-exports to mod.rs ----
m = open(path, encoding="utf-8").read()
inject = ""
for g in ["geom","config","paths"]:
    inject += "mod %s;\n" % g
m = m.replace("mod ops;\n", "mod ops;\n" + inject, 1)
# re-exports after the mod decls
reexp = ""
for g in ["geom","config","paths"]:
    reexp += "pub(in crate::bridge) use %s::*;\n" % g
m = m.replace("mod ops;\n" + inject, "mod ops;\n" + inject + reexp, 1)
open(path, "w", encoding="utf-8").write(m)
print("added mod decls + re-exports")
print("DONE")