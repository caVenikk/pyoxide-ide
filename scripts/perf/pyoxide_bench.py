#!/usr/bin/env python3
# Pyoxide measurements on the same generated files as the reference runs
# (replica/perf.md): start-up to the first frame, typing at line 50,000 of a
# 100,000-line file (3 x 200 keys, one every 50 ms, the text of
# pycharm_typing.py), opening a 10 MB file and scrolling it. Runs the release
# binary with PYOXIDE_BENCH; the window opens without taking the focus
# (PYOXIDE_BACKGROUND=1). Measure on an idle machine: the load average is
# printed first.
#
#     cargo build --release -p pyoxide-ide
#     python3 scripts/perf/pyoxide_bench.py --work <empty dir> [--runs 3]
import argparse, json, os, statistics, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pycharm_baseline as base

ROOT = Path(__file__).resolve().parents[2]


def run(binary: Path, args, bench: str, timeout: int):
    env = dict(os.environ, PYOXIDE_BACKGROUND="1", PYOXIDE_BENCH=bench)
    proc = subprocess.run([str(binary), *map(str, args)], env=env, capture_output=True,
                          text=True, timeout=timeout)
    markers = {}
    for line in proc.stdout.splitlines():
        if not line.startswith("PYOXIDE "):
            continue
        name, _, rest = line[len("PYOXIDE "):].partition(" ")
        try:
            markers[name] = json.loads(rest)
        except ValueError:
            markers[name] = rest
    if proc.returncode != 0:
        markers["exit"] = proc.returncode
    return markers


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--binary", type=Path, default=ROOT / "target/release/pyoxide-ide")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--only", choices=["startup", "typing", "open"])
    a = ap.parse_args()
    print("load average", " ".join(f"{v:.2f}" for v in os.getloadavg()), flush=True)
    work = a.work.resolve()
    results = {"startup": [], "typing": [], "open": []}
    kinds = [a.only] if a.only else ["startup", "typing", "open"]
    for kind in kinds:
        for _ in range(a.runs):
            # Fresh files: a typing run leaves the big file unchanged on disk,
            # but generating them again keeps every run identical.
            files = base.make_files(work)
            if kind == "startup":
                m = run(a.binary, [files["small"], files["small"] / "main.py"], "startup", 60)
                first = str(m.get("first_frame", "")).split("main_ms=")[-1]
                r = {"first_frame_ms": float(first) if first else None}
            elif kind == "typing":
                big = files["bigfile"]
                m = run(a.binary, [big, big / "big.py"], "typing", 300)
                r = {**m.get("typing", {"error": m}), **m.get("memory", {})}
            else:
                huge = files["hugefile"]
                m = run(a.binary, [huge], f"open:{huge / 'huge.py'}", 300)
                r = {**m.get("open", {"error": m}), "scroll": m.get("scroll"),
                     **m.get("memory", {})}
            results[kind].append(r)
            print(kind, json.dumps(r), flush=True)
    summary = {}
    if results["startup"]:
        vals = [r["first_frame_ms"] for r in results["startup"] if r.get("first_frame_ms")]
        summary["startup_first_frame_ms"] = (statistics.median(vals), max(vals)) if vals else None
    if results["open"]:
        for key in ("first_text_ms", "highlighted_ms"):
            vals = [r[key] for r in results["open"] if r.get(key) is not None]
            summary[f"open_{key}"] = (statistics.median(vals), max(vals)) if vals else None
    print(json.dumps(summary, indent=2))
    (work / "pyoxide-results.json").write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
