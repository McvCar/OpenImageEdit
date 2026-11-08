import re, json
path = "crates/ui/src/bridge/mod.rs"
lines = open(path, encoding="utf-8").read().split("\n")

# impl qobject::Bridge block
impl_start = None
for i, l in enumerate(lines):
    if l.strip() == "impl qobject::Bridge {":
        impl_start = i
        break
assert impl_start is not None
# find matching close
depth = 0
impl_end = None
for i in range(impl_start, len(lines)):
    depth += lines[i].count("{") - lines[i].count("}")
    if depth == 0:
        impl_end = i
        break
print("impl qobject::Bridge: L%d..L%d" % (impl_start+1, impl_end+1))

# methods: each starts at a line matching ^    (pub )?fn  OR a #[qproperty/#[qinvokable
# We collect: for each fn, the start (including preceding attrs) and end (brace balanced)
methods = []
i = impl_start + 1
while i < impl_end:
    l = lines[i]
    if re.match(r"^    (pub )?fn ", l):
        # walk back to include attributes
        s = i
        while s > impl_start and re.match(r"^    #\[", lines[s-1]):
            s -= 1
        # brace balance from the fn line
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
        methods.append((s+1, e+1, name))
        i = e + 1
    else:
        i += 1

print("methods: %d" % len(methods))
for a, b, n in methods:
    print("  L%d-%d (%d) %s" % (a, b, b-a+1, n))
json.dump([{"a":a,"b":b,"name":n} for a,b,n in methods], open("scripts/bridge_methods.json","w"), indent=0)
print("DONE")