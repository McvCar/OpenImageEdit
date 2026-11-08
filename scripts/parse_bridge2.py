import re, json
path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

impl_start = None
for i, l in enumerate(lines):
    if l.strip() == "impl qobject::Bridge {":
        impl_start = i
        break
assert impl_start is not None
depth = 0
impl_end = None
for i in range(impl_start, len(lines)):
    depth += lines[i].count("{") - lines[i].count("}")
    if depth == 0:
        impl_end = i
        break
print("impl qobject::Bridge: L%d..L%d" % (impl_start+1, impl_end+1))

methods = []
i = impl_start + 1
while i < impl_end:
    l = lines[i]
    if re.match(r"^    (pub )?fn ", l):
        is_pub = l.startswith("    pub fn")
        s = i
        while s > impl_start and re.match(r"^    #\[", lines[s-1]):
            s -= 1
        d = 0
        e = i
        started = False
        for j in range(i, impl_end+1):
            d += lines[j].count("{") - lines[j].count("}")
            if "{" in lines[j]:
                started = True
            if started and d == 0:
                e = j
                break
        name = re.match(r"^    (?:pub )?fn (\w+)", l).group(1)
        methods.append({"a": s+1, "b": e+1, "name": name, "pub": is_pub})
        i = e + 1
    else:
        i += 1

print("methods: %d (pub=%d, private=%d)" % (len(methods), sum(m["pub"] for m in methods), sum(not m["pub"] for m in methods)))
for m in methods:
    print("  L%d-%d %s %s" % (m["a"], m["b"], "pub" if m["pub"] else "fn ", m["name"]))
json.dump(methods, open("scripts/bridge_methods2.json","w"), indent=0)
print("impl_block_start=%d impl_block_end=%d" % (impl_start+1, impl_end+1))
print("DONE")