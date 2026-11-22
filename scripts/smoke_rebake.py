# -*- coding: utf-8 -*-
"""Rebake smoke: on the real game PSD, hide a styled group's members via the
panel-eye path and verify the baked shadow layers follow (pixel bytes change,
then restore after re-show).

Usage: python scripts/smoke_rebake.py
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

def layers(out):
    for line in out.splitlines():
        if '"event":"layers-dump"' in line:
            return json.loads(line[line.index("["): line.rindex("]") + 1])
    raise SystemExit("no layers-dump in output")

def dump_png(index, tag):
    path = os.path.join(TMP, f"smoke_rebake_{tag}_{index}.png")
    if os.path.exists(path):
        os.remove(path)
    run(f"layer-dump:{index}:{path}")
    if not os.path.exists(path):
        return None
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()

def main():
    ls = layers(run("layers-dump"))
    syn = [l for l in ls if l.get("syn")]
    print(f"synthetic rows in layers_json: {len(syn)}")
    assert syn, "expected synthetic rows (part 1 data)"
    gid = syn[0]["g"]
    shadows = [l for l in ls if l.get("syn") and l["g"] == gid]
    members = [l for l in ls if not l.get("syn") and l.get("g") == gid and l["i"] > 0]
    print(f"group {gid}: shadows={[(s['n'], s['x'], s['y'], s['w'], s['h']) for s in shadows]}")
    print(f"  members: {[(m['n'], m['i']) for m in members]}")
    assert shadows and members

    before_hash = {s["i"]: dump_png(s["i"], "before") for s in shadows}
    before_bounds = {s["i"]: (s["x"], s["y"], s["w"], s["h"]) for s in shadows}

    ops = ";".join(f"eye:{m['i']},0" for m in members)
    ls2 = layers(run(ops + ";layers-dump"))
    shadows2 = [l for l in ls2 if l.get("syn") and l["g"] == gid]
    after_hash = {s["i"]: dump_png(s["i"], "after") for s in shadows2}
    after_bounds = {s["i"]: (s["x"], s["y"], s["w"], s["h"]) for s in shadows2}

    changed = [i for i in before_hash
               if before_hash[i] != after_hash.get(i)
               or before_bounds[i] != after_bounds.get(i)]
    print(f"shadow layers that followed: {changed}")
    if not changed:
        print("FAIL: shadow did not follow member changes")
        sys.exit(1)

    ops2 = ";".join(f"eye:{m['i']},1" for m in members)
    ls3 = layers(run(ops2 + ";layers-dump"))
    shadows3 = [l for l in ls3 if l.get("syn") and l["g"] == gid]
    restored_hash = {s["i"]: dump_png(s["i"], "restored") for s in shadows3}
    restored_bounds = {s["i"]: (s["x"], s["y"], s["w"], s["h"]) for s in shadows3}
    ok = all(restored_hash.get(i) == before_hash[i]
             and restored_bounds.get(i) == before_bounds[i] for i in before_hash)
    print(f"restored after re-show: {ok}")
    sys.exit(0 if ok else 1)

main()
