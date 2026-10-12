#!/usr/bin/env python3
# Reference typing latency and 10 MB file opening in PyCharm on macOS
# (replica/perf.md), driven by the IDE's bundled performance-testing plugin:
# a script of %commands runs when the project opens, and the IDE writes its
# spans (typing, openFile, ...) to an OpenTelemetry JSON file.
#
#     python3 scripts/perf/pycharm_typing.py --work <empty dir> --kind typing
#     python3 scripts/perf/pycharm_typing.py --work <empty dir> --kind open
#
# Typing: open the 100,000-line file, go to line 50,000, then type the same
# 200 characters three times, one key every 50 ms. Open: open the 10 MB file
# from the project. PyCharm must not be running; it runs on a temporary copy
# of the user's configuration (see pycharm_baseline.py), removed afterwards.
import argparse, json, os, shutil, signal, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pycharm_baseline as base

# The text typed by both this runner and Pyoxide's typing measurement.
TEXT = ("# the quick brown fox jumps over the lazy dog and keeps typing to fill "
        "two hundred characters for the latency run of the reference editor ok done")


def write_script(path: Path, kind: str, runs: int, delay: int):
    lines = ["%waitForSmart"]
    if kind == "typing":
        text = (TEXT * 2)[:200]
        lines += ["%openFile -file big.py -unwfca", "%goto 50000 1", "%sleep 3000"]
        for _ in range(runs):
            lines += [f"%delayType {delay}|{text}", "%sleep 2000"]
    else:
        lines += ["%sleep 3000", "%openFile -file huge.py -unwfca", "%sleep 3000"]
    lines += ["%exitApp true"]
    path.write_text("\n".join(lines) + "\n")


def spans(path: Path):
    out = []
    if not path.exists():
        return out
    try:
        data = json.loads(path.read_text(errors="replace"))
    except ValueError:
        return out

    def walk(o):
        if isinstance(o, dict):
            if "operationName" in o or ("name" in o and "duration" in o):
                out.append(o)
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for x in o:
                walk(x)

    walk(data)
    return out


def run(app: Path, vm: Path, project: Path, script: Path, trace: Path, timeout: int):
    opts = [o for o in vm.read_text().splitlines()
            if not o.startswith(("-Dtestscript.filename", "-Didea.diagnostic.opentelemetry.file"))]
    opts += [f"-Dtestscript.filename={script}", f"-Didea.diagnostic.opentelemetry.file={trace}"]
    vm.write_text("\n".join(opts) + "\n")
    if trace.exists():
        trace.unlink()
    before = base.app_pids(app)
    env = dict(os.environ, PYCHARM_VM_OPTIONS=str(vm))
    proc = subprocess.Popen([str(app / "Contents/MacOS/pycharm"), str(project)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        proc.wait(timeout)
    except subprocess.TimeoutExpired:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(60)
        except subprocess.TimeoutExpired:
            proc.kill()
    time.sleep(3)
    # Helpers such as the embedded browser server may outlive the IDE.
    for pid in base.app_pids(app) - before:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    return spans(trace)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--kind", choices=["typing", "open"], required=True)
    ap.add_argument("--app", type=Path)
    ap.add_argument("--config", type=Path)
    ap.add_argument("--runs", type=int, default=3, help="typing runs of 200 keys")
    ap.add_argument("--delay", type=int, default=50, help="ms between keys")
    ap.add_argument("--timeout", type=int, default=600)
    a = ap.parse_args()
    if base.running():
        sys.exit("PyCharm is running; close it first")
    app, config = a.app or base.find_app(), a.config or base.find_config()
    work = a.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    projects = base.make_files(work)
    prof, vm = base.prepare_profile(work, app, config)
    try:
        script = work / f"{a.kind}.script"
        write_script(script, a.kind, a.runs, a.delay)
        project = projects["bigfile" if a.kind == "typing" else "hugefile"]
        trace = work / f"{a.kind}-trace.json"
        found = run(app, vm, project, script, trace, a.timeout)
    finally:
        shutil.rmtree(prof / "config", ignore_errors=True)  # holds a license copy
    keep = {"typing", "openFile", "firstCodeAnalysis"}
    for s in found:
        if (s.get("operationName") or s.get("name")) in keep:
            print(json.dumps(s, ensure_ascii=False)[:2000])
    if not found:
        print(f"no spans in {trace}; see {prof / 'log/idea.log'}")


if __name__ == "__main__":
    main()
