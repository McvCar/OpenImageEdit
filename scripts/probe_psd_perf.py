#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""PSD layered-editing performance probe (M0 baseline + M6 hard gates).

`--baseline`
    Runs `dec_bench --json` over the generated fixtures three times and stores
    the median `psd-stage1-ms` in tmp/psd-baseline.json. Run this BEFORE any
    implementation change so the "no regression" claim is measurable.

`--collect`
    Launches real debug instances and captures the structured events into
    tmp/psd-perf-events.json: a layered open (stage 1 / stage 2 / lock), a
    100-layer open (thumbnail batches), a large-document PSD encode, and an
    oversized (>8192) read-only write-back rejection. Then runs `--check`
    in strict mode.

`--check`
    Reads structured UI events (JSON array or JSON Lines) and applies the
    hard thresholds from docs/PSD分层可编辑-B方案.md §6.4 / §6.2.1:
      * psd-stage2-ms (== editing lock duration) <= 3000 ms
      * psd-encode-ms <= 2000 ms
      * max(layer-thumb-first-ms, layer-thumb-batch-ms) <= 16 ms
      * psd-stage1-ms deviation from the stored baseline <= 3% (small fixture)
      * editingLocked must end in the unlocked state for every open
      * C8: a downsampled open must reject write-back

Usage (from a shell that already sourced scripts/env.ps1):
    python scripts/probe_psd_perf.py --baseline
    python scripts/probe_psd_perf.py --collect
    python scripts/probe_psd_perf.py --check --events tmp/psd-perf-events.json
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE_DIR = ROOT / "tmp" / "psd-fixtures"
BASELINE = ROOT / "tmp" / "psd-baseline.json"
EVENTS = ROOT / "tmp" / "psd-perf-events.json"
EXE = ROOT / "target" / "debug" / "imageedit.exe"
WORK = ROOT / "tmp" / "psd-perf"

STAGE2_LIMIT_MS = 3000
ENCODE_LIMIT_MS = 2000
THUMB_LIMIT_MS = 16
STAGE1_DRIFT = 0.03
BASELINE_FIXTURES = ("single-layer.psd", "small-layers.psd", "large-10layers.psd")


def run_bench(path, repeat):
    values = []
    meta = None
    for _ in range(repeat):
        proc = subprocess.run(
            ["cargo", "run", "-q", "--example", "dec_bench", "--", str(path), "--json"],
            cwd=str(ROOT),
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError("dec_bench failed for %s: %s" % (path, proc.stderr.strip()[-400:]))
        line = next((l for l in proc.stdout.splitlines() if l.lstrip().startswith("{")), None)
        if not line:
            raise RuntimeError("dec_bench produced no JSON for %s: %s" % (path, proc.stdout[-400:]))
        data = json.loads(line)
        values.append(data["stage1_ms"])
        meta = data
    meta["stage1_ms"] = int(statistics.median(values))
    meta["samples"] = values
    return meta


def baseline(repeat):
    missing = [f for f in BASELINE_FIXTURES if not (FIXTURE_DIR / f).exists()]
    if missing:
        print("FAIL missing fixtures: %s (run make_psd_fixture.py --only all first)" % missing)
        return 1
    out = {"schema": 1, "note": "stage1 = composite-only decode; pre-implementation baseline", "fixtures": {}}
    for name in BASELINE_FIXTURES:
        path = FIXTURE_DIR / name
        data = run_bench(path, repeat)
        out["fixtures"][name] = {
            "stage1_ms": data["stage1_ms"],
            "samples": data["samples"],
            "width": data["width"],
            "height": data["height"],
            "file_bytes": data["file_bytes"],
        }
        print("baseline %-24s %5d ms  (%dx%d, %d bytes)" % (name, data["stage1_ms"], data["width"], data["height"], data["file_bytes"]))
    BASELINE.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print("saved -> %s" % BASELINE)
    return 0


def load_events(path):
    text = Path(path).read_text(encoding="utf-8")
    try:
        data = json.loads(text)
        if isinstance(data, dict):
            data = data.get("events", [data])
        return data if isinstance(data, list) else []
    except json.JSONDecodeError:
        return [json.loads(l) for l in text.splitlines() if l.strip().startswith("{")]


def metric(events, name):
    vals = [e.get("value", e.get("ms")) for e in events if e.get("event") == name]
    vals = [v for v in vals if isinstance(v, (int, float))]
    return max(vals) if vals else None


def check(events_path, strict=False):
    if not Path(events_path).exists():
        print("FAIL events file not found: %s" % events_path)
        return 1
    events = load_events(events_path)
    failures = []

    def gate(name, limit, label):
        v = metric(events, name)
        if v is None:
            msg = "%s missing (UI events not captured)" % name
            if strict:
                failures.append(msg)
            else:
                print("WARN " + msg)
        elif v > limit:
            failures.append("%s %d > %d" % (name, v, limit))
        else:
            print("PASS %s = %d ms (<= %d)" % (name, v, limit))
        return v

    gate("psd-stage2-ms", STAGE2_LIMIT_MS, "editing lock")
    gate("psd-encode-ms", ENCODE_LIMIT_MS, "PSD encode")

    # C6: the first expansion AND every <=8-row batch are UI-thread work.
    first = metric(events, "layer-thumb-first-ms")
    batch = metric(events, "layer-thumb-batch-ms")
    thumb = max([v for v in (first, batch) if v is not None], default=None)
    if thumb is None:
        msg = "layer-thumb-* missing"
        if strict:
            failures.append(msg)
        else:
            print("WARN " + msg)
    elif thumb > THUMB_LIMIT_MS:
        failures.append("layer-thumb UI occupancy %d > %d (first=%s batch=%s)"
                        % (thumb, THUMB_LIMIT_MS, first, batch))
    else:
        print("PASS layer-thumb UI occupancy = %d ms (<= %d, first=%s batch=%s)"
              % (thumb, THUMB_LIMIT_MS, first, batch))

    # Editing lock must always end unlocked; every "locked" needs a matching
    # "unlocked" (C2-c: failure / skip / switch / handoff paths included).
    locks = [e for e in events if e.get("event") == "editing-locked"]
    unlocks = [e for e in events if e.get("event") == "editing-unlocked"]
    if locks:
        if len(unlocks) < len(locks):
            failures.append("editing lock leak: %d locks vs %d unlocks" % (len(locks), len(unlocks)))
        else:
            print("PASS editing lock lifecycle: %d locks / %d unlocks" % (len(locks), len(unlocks)))
    elif strict:
        failures.append("no editing-locked events captured")
    else:
        print("WARN no editing-locked events")

    # C8: an oversized downsampled document must reject write-back.
    if any(e.get("event") == "downsampled" for e in events):
        rejected = any(
            e.get("event") == "save-error" and "降采样" in str(e.get("detail", ""))
            for e in events
        )
        if rejected:
            print("PASS C8 downsampled write-back rejected")
        else:
            failures.append("C8: downsampled PSD write-back was not rejected")

    # No regression on the composite-only first frame.
    stage1 = metric(events, "psd-stage1-ms")
    if stage1 is not None and BASELINE.exists():
        base = json.loads(BASELINE.read_text(encoding="utf-8"))["fixtures"]
        small = base.get("small-layers.psd", {}).get("stage1_ms", 0)
        if small > 0:
            limit = max(3, small * (1 + STAGE1_DRIFT))
            if stage1 > limit:
                failures.append("psd-stage1-ms %d > %.1f (baseline %d + 3%%)" % (stage1, limit, small))
            else:
                print("PASS psd-stage1-ms = %d ms (baseline %d, limit %.1f)" % (stage1, small, limit))

    if failures:
        for f in failures:
            print("FAIL " + f)
        return 1
    print("ALL PERFORMANCE GATES PASS")
    return 0


def capture(args, needle, timeout=40.0, settle=1.0, extra_env=None):
    """Run one debug instance, collect stdout until `needle` shows up."""
    e = dict(os.environ)
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    e["PATH"] = "D:/Qt/6.8.3/msvc2022_64/bin;" + e.get("PATH", "")
    for key in ("IMAGEEDIT_PSD_STAGE2_DELAY_MS", "IMAGEEDIT_PSD_STAGE2_FAIL",
                "IMAGEEDIT_PSD_AUTO_SKIP", "IMAGEEDIT_PSD_SYNC_COMPOSITE"):
        e.pop(key, None)
    if extra_env:
        e.update(extra_env)
    proc = subprocess.Popen(
        [str(EXE)] + args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, errors="replace", env=e, bufsize=1,
    )
    lines, seen = [], threading.Event()

    def reader():
        for raw in proc.stdout:
            line = raw.rstrip("\n")
            lines.append(line)
            if needle in line:
                seen.set()
        seen.set()

    threading.Thread(target=reader, daemon=True).start()
    seen.wait(timeout)
    time.sleep(settle)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
    return lines


def json_events(lines):
    out = []
    for line in lines:
        at = line.find('{"phase"')
        if at < 0:
            continue
        try:
            obj, _ = json.JSONDecoder().raw_decode(line[at:])
        except ValueError:
            continue
        out.append(obj)
    return out


def collect():
    needed = ("small-layers.psd", "many-100layers.psd",
              "large-10layers.psd", "oversize.psd")
    missing = [f for f in needed if not (FIXTURE_DIR / f).exists()]
    if missing:
        print("FAIL missing fixtures: %s (run make_psd_fixture.py --only all)" % missing)
        return 1
    if not EXE.exists():
        print("FAIL debug exe not found:", EXE)
        return 1
    WORK.mkdir(parents=True, exist_ok=True)
    collected = []

    def run(label, argv, needle, settle=1.0, extra_env=None):
        print("collect %-24s ..." % label, flush=True)
        lines = capture(argv, needle, settle=settle, extra_env=extra_env)
        events = json_events(lines)
        collected.extend(events)
        print("  %d events" % len(events))
        return events

    # C1/C2/C3: layered open, stage1 + stage2 + lock lifecycle.
    run("layered open", [str(FIXTURE_DIR / "small-layers.psd")],
        '"event":"editing-unlocked"', settle=1.0)
    # C6/C7: 100 layers, all thumbnail batches must drain.
    run("100-layer open", [str(FIXTURE_DIR / "many-100layers.psd")],
        '"event":"editing-unlocked"', settle=1.8)
    # C5: encode a real 12MP-ish / 10-layer document on the worker thread.
    big = WORK / "encode.psd"
    shutil.copyfile(FIXTURE_DIR / "large-10layers.psd", big)
    run("large PSD encode", [str(big), "--script=save"],
        '"event":"saved"', settle=1.0)
    # C8: force downsampling-first on an oversized PSD, then attempt Ctrl+S.
    cfg_dir = WORK / "appdata" / "ImageEdit"
    cfg_dir.mkdir(parents=True, exist_ok=True)
    (cfg_dir / "settings.ini").write_text("max_dimension=8192\n", encoding="utf-8")
    huge = WORK / "oversize.psd"
    shutil.copyfile(FIXTURE_DIR / "oversize.psd", huge)
    run("C8 read-only guard",
        [str(huge), "--script=save"], '"event":"save-error"', settle=1.0,
        extra_env={"APPDATA": str(WORK / "appdata")})

    EVENTS.write_text(
        "\n".join(json.dumps(e, ensure_ascii=False) for e in collected) + "\n",
        encoding="utf-8",
    )
    print("saved %d events -> %s" % (len(collected), EVENTS))
    return check(EVENTS, strict=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", action="store_true")
    ap.add_argument("--collect", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--events", default=str(EVENTS))
    ap.add_argument("--repeat", type=int, default=3)
    args = ap.parse_args()
    if args.baseline:
        return baseline(args.repeat)
    if args.collect:
        return collect()
    if args.check:
        return check(args.events)
    ap.print_help()
    return 0


if __name__ == "__main__":
    sys.exit(main())
