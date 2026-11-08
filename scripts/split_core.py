import re

path = "crates/core/src/lib.rs"
lines = open(path, encoding="utf-8").read().split("\n")
N = len(lines)
print("total lines", N)

def seg(a, b):
    """1-based inclusive -> 0-based slice lines[a-1:b]"""
    return lines[a-1:b]

# segments (1-based inclusive)
geometry = seg(10, 173)
mark = seg(174, 666)
layer = seg(667, 700)
document_struct = seg(701, 737)
editerror = seg(738, 757)
transform = seg(758, 854)
doc_snapshot_and_impl = seg(855, 897)
op = seg(898, 1266)
ops = seg(1267, 1434)
document_impl = seg(1435, 1857)
tests = seg(1858, N)

# sanity: first/last lines
def show(name, s):
    first = s[0].strip()[:40] if s else ""
    last = s[-1].strip()[:40] if s else ""
    print("%-16s %3d lines  first=%r last=%r" % (name, len(s), first, last))

show("geometry", geometry)
show("mark", mark)
show("layer", layer)
show("document_struct", document_struct)
show("editerror", editerror)
show("transform", transform)
show("doc_snapshot_and_impl", doc_snapshot_and_impl)
show("op", op)
show("ops", ops)
show("document_impl", document_impl)
show("tests", tests)

R = "use image::RgbaImage;\n"

def write(fname, body, with_rgba=False):
    header = "// %s\n" % fname
    if with_rgba:
        header += R
    header += "use super::*;\n\n"
    open(fname, "w", encoding="utf-8").write(header + "\n".join(body) + "\n")
    print("wrote", fname, len(body), "lines")

write("crates/core/src/geometry.rs", geometry)
write("crates/core/src/mark.rs", mark, with_rgba=True)
write("crates/core/src/layer.rs", layer, with_rgba=True)
write("crates/core/src/transform.rs", transform, with_rgba=True)
write("crates/core/src/op.rs", editerror + [""] + op, with_rgba=False)
write("crates/core/src/ops.rs", ops, with_rgba=True)
# document/mod.rs: struct + snapshot/impl + main impl
doc_body = document_struct + [""] + doc_snapshot_and_impl + [""] + document_impl
write("crates/core/src/document/mod.rs", doc_body, with_rgba=True)

# ---- rewrite lib.rs: keep 1-9 (use+mod+pub use) then append mod decls + pub use + tests ----
head = lines[0:9]  # 1-9
moddecls = [
    "mod geometry;",
    "mod mark;",
    "mod layer;",
    "mod transform;",
    "mod op;",
    "mod ops;",
    "mod document;",
]
pubuses = [
    "pub use geometry::{Size, Rect, Selection};",
    "pub use mark::{MarkKind, Mark};",
    "pub use layer::Layer;",
    "pub use transform::Transform;",
    "pub use op::{EditError, EditOp};",
    "pub use document::Document;",
]
new = head + [""] + moddecls + [""] + pubuses + [""] + tests
open(path, "w", encoding="utf-8").write("\n".join(new))
print("lib.rs now", len(new), "lines")
print("DONE")