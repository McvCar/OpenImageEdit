#!/usr/bin/env python
"""Verify the layered-editing "native preview calibration" (docs section 17).

Three independent launches of the same PSD:

  fidelity : default open, composite-only (ground truth frame).
  layered  : enter layered editing; the frame must still be the composite.
  edited   : simulate one edit; calibration must turn off (sticky).

The strongest assertion is a pixel comparison between `fidelity` and `layered`
preview BMPs: both come from the same PreviewComposer + same PSD composite, so
they must be byte-identical (MAE 0.00). We also read `calibration-mae`, which
measures the frame that was replaced (the 617-layer reconstruction).

No clicks / SendInput: everything is driven by structured log events and the
BMP frames the app writes into %TEMP%.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / "target" / "debug" / "imageedit.exe"
QT_BIN = Path(os.environ.get("IMAGEEDIT_QT_ROOT", "C:/Qt/6.8.3/msvc2022_64/bin"))
DEFAULT_PSD = "D:/_APP_CODE/_XhCompany/FontGame/Res/0302-界面UI-1.psd"
TMP = Path(tempfile.gettempdir())
OUT = Path("C:/_APP_CODE")


def base_env(extra):
    e = dict(os.environ)
    e["PATH"] = str(QT_BIN) + os.pathsep + e.get("PATH", "")
    e["QT_FORCE_STDERR_LOGGING"] = "1"
    for key in (
        "IMAGEEDIT_FIDELITY_PROBE",
        "IMAGEEDIT_CALIBRATION_MAE",
        "IMAGEEDIT_CALIBRATION_PROBE",
        "IMAGEEDIT_AUTO_LAYERED",
    ):
        e.pop(key, None)
    e.update(extra or {})
    return e


def kill_all():
    subprocess.run(
        ["taskkill", "/F", "/IM", "imageedit.exe"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1.0)


def clear_frames():
    for p in TMP.glob("imageedit-preview-*.bmp"):
        try:
            p.unlink()
        except OSError:
            pass


def newest_frame():
    frames = sorted(TMP.glob("imageedit-preview-*.bmp"), key=lambda p: p.stat().st_mtime)
    return frames[-1] if frames else None


def run_variant(name, psd, extra, marker, seconds):
    kill_all()
    clear_frames()
    log_path = OUT / ("cal_%s.log" % name)
    frame_path = OUT / ("cal_%s.bmp" % name)
    if frame_path.exists():
        frame_path.unlink()
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        proc = subprocess.Popen(
            [str(EXE), str(psd)],
            cwd=str(ROOT),
            stdout=log,
            stderr=subprocess.STDOUT,
            env=base_env(extra),
        )
        deadline = time.time() + seconds
        text = ""
        hit = False
        while time.time() < deadline:
            time.sleep(0.5)
            text = log_path.read_text(encoding="utf-8", errors="replace")
            if marker in text:
                hit = True
                # Let the corresponding frame land on disk AND let the tail
                # events flush: `fidelity-mode` is printed after the first
                # calibration frame, so the snapshot taken at the marker tick
                # can still be missing it (flaky assertion, not a bug).
                time.sleep(1.5)
                text = log_path.read_text(encoding="utf-8", errors="replace")
                break
        frame = newest_frame()
        if frame:
            shutil.copyfile(frame, frame_path)
        kill_all()
        try:
            proc.wait(timeout=5)
        except Exception:
            pass
    return {
        "name": name,
        "marker": marker,
        "hit": hit,
        "frame": frame_path if frame_path.exists() else None,
        "log": text,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--psd", default=DEFAULT_PSD)
    ap.add_argument("--seconds", type=int, default=40)
    args = ap.parse_args()
    psd = Path(args.psd)
    if not EXE.exists():
        print("FAIL missing exe: %s" % EXE)
        return 1
    if not psd.exists():
        print("FAIL missing psd: %s" % psd)
        return 1

    variants = [
        (
            "fidelity",
            {},
            '"event":"fidelity-state","value":true',
        ),
        (
            "layered",
            {
                "IMAGEEDIT_FIDELITY_PROBE": "enter",
                "IMAGEEDIT_CALIBRATION_MAE": "1",
            },
            '"event":"calibration-frame"',
        ),
        (
            "edited",
            {
                "IMAGEEDIT_FIDELITY_PROBE": "enter",
                "IMAGEEDIT_CALIBRATION_MAE": "1",
                "IMAGEEDIT_CALIBRATION_PROBE": "edit",
            },
            '"event":"calibration-off"',
        ),
    ]
    results = {}
    for name, extra, marker in variants:
        r = run_variant(name, psd, extra, marker, args.seconds)
        results[name] = r
        print("[%s] marker=%s hit=%s frame=%s" % (name, marker, r["hit"], bool(r["frame"])))

    checks = []

    def check(label, ok, detail=""):
        checks.append((label, bool(ok), detail))

    fid = results["fidelity"]
    lay = results["layered"]
    edt = results["edited"]

    check("fidelity frame exists", fid["frame"] is not None)
    check("layered frame exists", lay["frame"] is not None)
    check("layered calibration installed", '"event":"calibration-install","active":true' in lay["log"])
    check("layered entered editing", '"event":"fidelity-mode","value":false' in lay["log"])
    check("edit turns calibration off", '"event":"calibration-off"' in edt["log"])
    check("fidelity has no calibration", '"event":"calibration-install"' not in fid["log"])

    m = re.search(r'"event":"calibration-mae","value":([0-9.]+)', lay["log"])
    replaced_mae = float(m.group(1)) if m else None
    check("rebuilt frame had real error", replaced_mae is not None and replaced_mae > 1.0,
          "calibration-mae=%s" % replaced_mae)

    if fid["frame"] and lay["frame"]:
        import numpy as np
        from PIL import Image

        a = np.asarray(Image.open(fid["frame"]).convert("RGB"), dtype=np.int16)
        b = np.asarray(Image.open(lay["frame"]).convert("RGB"), dtype=np.int16)
        same_shape = a.shape == b.shape
        check("preview sizes match", same_shape, "%s vs %s" % (a.shape, b.shape))
        if same_shape:
            mae = float(np.abs(a - b).mean())
            mx = int(np.abs(a - b).max())
            check("layered == fidelity (MAE 0.00)", mae == 0.0, "MAE=%.4f max=%d" % (mae, mx))
            print("FRAME COMPARE: shape=%s MAE=%.4f max=%d" % (a.shape, mae, mx))

    print("\n=== checks ===")
    for label, ok, detail in checks:
        print("  %-38s %s %s" % (label, "PASS" if ok else "FAIL", detail))
    n_pass = sum(1 for _, ok, _ in checks if ok)
    print("\n%d/%d PASS" % (n_pass, len(checks)))
    print(json.dumps({
        "replaced_mae": replaced_mae,
        "passed": n_pass,
        "total": len(checks),
    }, ensure_ascii=False))
    return 0 if n_pass == len(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
