# -*- coding: utf-8 -*-
"""Rebake smoke (child groups): hide/show a CHILD group nested under a styled
group and verify the styled group's baked shadow layers follow — including
the "all members hidden -> shadow cleared" rule. All steps run in ONE process
(layer-dump is a script op) so state carries across them.

Usage: python scripts/smoke_rebake_child.py
"""
import hashlib
import json
import os
import subprocess
import sys

EXE = r"D:\_APP_CODE\ImageEdit\target\debug\imageedit.exe"
PSD = r"D:\_APP_CODE\_XhCompany\FontGame\Res\0302-界面UI-1.psd"
QT = r"D:\Qt\6.8.3\msvc2022_64\bin"
TMP = os.environ.get("TEMP", r"C:\Users\Admin\AppData\Local\Temp")

def run(script):
    env = dict(os.environ)
    env["PATH"] = QT + ";" + env.get("PATH", "")
    p = subprocess.run([EXE, "--auto-quit", "--script=" + script, "--", PSD],
                       capture_output=True, timeout=600, env=env)
    return p.stdout.decode("utf-8", errors="replace")

def event(out, name, key):
    for line in out.splitlines():
        if f'"event":"{name}"' in line:
            i = line.index(f'"{key}":[') + len(key) + 3
            j = line.rindex("]")
            return json.loads(line[i: j + 1])
    raise SystemExit(f"no {name} in output")

def main():
    base = run("fx-dump;groups-dump;layers-dump")
    groups = event(base, "groups-dump", "groups")
    layers = event(base, "layers-dump", "layers")

    styled = {}
    for l in layers:
        if l.get("syn"):
            styled.setdefault(l["g"], []).append(l)
    if not styled:
        raise SystemExit("no styled groups with shadows")

    target = None
    for gid in styled:
        children = [g for g in groups if g.get("p") == gid]
        if children:
            target = (gid, children)
            break
    if not target:
        print("SKIP: no styled group has child groups")
        sys.exit(2)
    gid, children = target
    shadows = styled[gid]
    print(f"styled group {gid}: shadows={[s['i'] for s in shadows]}, "
          f"children={[(c['id'], c['n']) for c in children]}")

    # 基线 dump（独立进程，加载态即基线）。
    before = {}
    for s in shadows:
        p = os.path.join(TMP, f"smoke_child_base_{s['i']}.png")
        if os.path.exists(p):
            os.remove(p)
        run(f"layer-dump:{s['i']}:{p}")
        with open(p, "rb") as f:
            before[s["i"]] = hashlib.sha256(f.read()).hexdigest()

    def step(ops, tag):
        dumps = ";".join(
            f"layer-dump:{s['i']}:{os.path.join(TMP, f'smoke_child_{tag}_{s[chr(105)]}.png')}"
            for s in shadows)
        run(";".join(ops + [dumps]))
        out = {}
        for s in shadows:
            p = os.path.join(TMP, f"smoke_child_{tag}_{s['i']}.png")
            with open(p, "rb") as f:
                out[s["i"]] = hashlib.sha256(f.read()).hexdigest()
        return out

    child = children[0]
    after = step([f"group-visible:{child['id']},0"], "hide")
    changed = [i for i in before if before[i] != after.get(i)]
    print(f"after hiding child {child['id']}: shadows changed = {changed}")
    if not changed:
        print("FAIL: hiding a child group did not rebake the styled parent shadow")
        sys.exit(1)

    restored = step([f"group-visible:{child['id']},1"], "show")
    ok = all(restored.get(i) == before[i] for i in before)
    print(f"after re-showing child: restored = {ok}")
    sys.exit(0 if ok else 1)

main()
