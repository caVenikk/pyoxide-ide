#!/usr/bin/env python3
# Pyoxide measurements on the same generated files as the reference runs
# (replica/perf.md): start-up to the first frame, typing at line 50,000 of a
# 100,000-line file (3 x 200 keys, one every 50 ms, the text of
# pycharm_typing.py), opening a 10 MB file and scrolling it, and the
# completion popup in the 200-line file against a service that answers after
# a fixed latency (--latency, ms). Runs the release
# binary with PYOXIDE_BENCH; the window opens without taking the focus
# (PYOXIDE_BACKGROUND=1). Measure on an idle machine: the load average is
# printed first. `--marks N` shows N problems spread over the big file during
# the typing runs (underlines, error stripe, inspection widget).
# `--soft-wraps` opens every file with soft wraps (an `editor.toml` in a
# configuration folder of its own); `--only typing-long` types at line
# 50,000 of a generated file of 52,000 long lines (120 to 520 chars).
#
#     cargo build --release -p pyoxide-ide
#     python3 scripts/perf/pyoxide_bench.py --work <empty dir> [--runs 3] [--marks 1000]
#         [--only completion --latency 50] [--soft-wraps]
import argparse, json, os, statistics, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pycharm_baseline as base

ROOT = Path(__file__).resolve().parents[2]


# The configuration folder of the runs (`editor.toml`), set in main().
CONFIG = None


def long_lines(work: Path) -> Path:
    # 52,000 lines of words, 120 to 520 chars each (about 17 MB).
    folder = work / "projects/longlines"
    folder.mkdir(parents=True, exist_ok=True)
    words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu".split()
    lines = []
    for i in range(52_000):
        width = 120 + (i * 37) % 400
        out, k, n = [], i, 0
        while n < width:
            out.append(words[k % len(words)])
            n += len(out[-1]) + 1
            k += 7
        lines.append("    # " + " ".join(out))
    (folder / "long.py").write_text("\n".join(lines) + "\n")
    return folder


def run(binary: Path, args, bench: str, timeout: int, marks: int = 0):
    env = dict(os.environ, PYOXIDE_BACKGROUND="1", PYOXIDE_BENCH=bench,
               PYOXIDE_BENCH_MARKS=str(marks))
    if CONFIG is not None:
        env["PYOXIDE_CONFIG_DIR"] = str(CONFIG)
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
    ap.add_argument("--only", choices=["startup", "typing", "typing-long", "open", "completion"])
    ap.add_argument("--marks", type=int, default=0)
    ap.add_argument("--latency", type=int, default=50)
    ap.add_argument("--soft-wraps", action="store_true")
    a = ap.parse_args()
    print("load average", " ".join(f"{v:.2f}" for v in os.getloadavg()), flush=True)
    work = a.work.resolve()
    global CONFIG
    CONFIG = work / "config"
    CONFIG.mkdir(parents=True, exist_ok=True)
    soft_wraps = '[soft_wraps]\nenabled = true\nfiles = "*"\n'
    (CONFIG / "editor.toml").write_text(soft_wraps if a.soft_wraps else "")
    results = {"startup": [], "typing": [], "typing-long": [], "open": [], "completion": []}
    kinds = [a.only] if a.only else ["startup", "typing", "open", "completion"]
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
                m = run(a.binary, [big, big / "big.py"], "typing", 300, a.marks)
                r = {**m.get("typing", {"error": m}), **m.get("memory", {})}
            elif kind == "typing-long":
                folder = long_lines(work)
                m = run(a.binary, [folder, folder / "long.py"], "typing", 300, a.marks)
                r = {**m.get("typing", {"error": m}), **m.get("memory", {})}
            elif kind == "completion":
                small = files["small"]
                m = run(a.binary, [small, small / "main.py"], f"completion:{a.latency}", 300)
                r = {**m.get("completion", {"error": m}), **m.get("memory", {})}
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
