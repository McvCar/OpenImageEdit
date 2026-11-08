import re

path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

def find_item(name, kind="fn"):
    for i, l in enumerate(lines):
        if kind == "fn" and re.match(r"^fn %s\(" % re.escape(name), l):
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
            return i, i
    return None

def find_impl_for(typ):
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

plan = {
    "geom": [("fn","fit_zoom"),("fn","display_size"),("fn","size_close"),
             ("fn","preview_target"),("fn","selection_after_removal")],
    "config": [("struct","AppConfig"),("impl","AppConfig"),("fn","human_size")],
    "paths": [("fn","clipboard"),("fn","write_checker_tile_bmp"),("fn","file_url"),
              ("fn","extension_for"),("fn","is_psd_path"),("const","PSD_SAVE_HINT"),
              ("fn","save_format_for_source")],
}

ranges = []
for g, items in plan.items():
    for kind, name in items:
        se = find_impl_for(name) if kind == "impl" else find_item(name, kind)
        assert se, "not found %s %s" % (kind, name)
        s, e = se
        ranges.append((s, e, "%s.%s" % (g, name)))
        print("  L%d-%d %s" % (s+1, e+1, name))

# Delete [s, e] INCLUSIVE (include closing brace), descending order.
# Also drop one trailing blank line after e to avoid double blanks.
for s, e, _ in sorted(ranges, key=lambda x: x[0], reverse=True):
    end = e + 1  # exclusive end, includes closing brace
    while end < len(lines) and lines[end].strip() == "":
        end += 1
    del lines[s:end]

new_mod = "\n".join(lines)
open(path, "w", encoding="utf-8").write(new_mod)
print("mod.rs now %d lines" % len(lines))

# add mod decls + re-exports
m = open(path, encoding="utf-8").read()
inject = "mod geom;\nmod config;\nmod paths;\n"
reexp = "pub(in crate::bridge) use geom::*;\npub(in crate::bridge) use config::*;\npub(in crate::bridge) use paths::*;\n"
m = m.replace("mod ops;\n", "mod ops;\n" + inject + reexp, 1)
open(path, "w", encoding="utf-8").write(m)
print("added mod decls + re-exports")
print("DONE")