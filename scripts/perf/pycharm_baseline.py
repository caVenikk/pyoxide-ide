#!/usr/bin/env python3
# Reference baseline of PyCharm on macOS (replica/perf.md, issue #11).
#
#     python3 scripts/perf/pycharm_baseline.py --work <empty dir>
#
# Runs a licensed PyCharm installation against generated test projects and
# reads the IDE's own log: time until the project is ready (first "exit dumb
# mode") and indexed (no dumb mode for 15 s after an exit), and the resident
# memory of the process tree after an idle period. PyCharm must not be
# running. The IDE runs on a temporary copy of the user's configuration with
# its own cache and log directories, so the real profile is never modified.
# Typing latency and opening a 10 MB file need input simulation and are not
# measured here.
import argparse, json, os, re, shutil, signal, statistics, subprocess, sys, time
from pathlib import Path
from xml.etree import ElementTree as ET

LOG_TS = re.compile(r"^\S+ \S+ \[\s*(\d+)\]")


def find_app():
    for base in (Path.home() / "Applications", Path("/Applications")):
        for name in ("PyCharm.app", "PyCharm Professional Edition.app", "PyCharm CE.app"):
            if (base / name).exists():
                return base / name
    sys.exit("PyCharm.app not found; pass --app")


def find_config():
    root = Path.home() / "Library/Application Support/JetBrains"
    dirs = sorted(p for p in root.glob("PyCharm*") if (p / "options").is_dir())
    if not dirs:
        sys.exit("no PyCharm configuration directory found; pass --config")
    return dirs[-1]


def running():
    out = subprocess.run(["pgrep", "-f", "PyCharm.*\\.app/Contents/MacOS/pycharm"],
                         capture_output=True, text=True).stdout
    return bool(out.strip())


def make_projects(work: Path, mid_tag: str):
    small = work / "projects/small"
    small.mkdir(parents=True, exist_ok=True)
    (small / "main.py").write_text("".join(
        f"def f{i}(x: int) -> int:\n    return x * {i} + 1\n\n\n" for i in range(50)))
    big = work / "projects/bigfile"
    big.mkdir(parents=True, exist_ok=True)
    lines = []
    for i in range(25_000):
        lines += [f"def func_{i}(a, b=None):", f"    value = a + {i}", "    return value", ""]
    (big / "big.py").write_text("\n".join(lines) + "\n")
    huge = work / "projects/hugefile"
    huge.mkdir(parents=True, exist_ok=True)
    row = "x = " + "'" + "a" * 90 + "'\n"
    (huge / "huge.py").write_text(row * (10 * 1024 * 1024 // len(row)))
    mid = work / "projects/flask"
    if not mid.exists():
        subprocess.run(["git", "clone", "--quiet", "--depth", "1", "--branch", mid_tag,
                        "https://github.com/pallets/flask.git", str(mid)], check=True)
    return {"small": small, "bigfile": big, "hugefile": huge, "mid": mid}


def prepare_profile(work: Path, app: Path, user_config: Path):
    prof = work / "ide"
    if prof.exists():
        shutil.rmtree(prof)
    config = prof / "config"
    shutil.copytree(user_config, config, symlinks=True,
                    ignore=shutil.ignore_patterns("*.lock", "port*"))
    # Trust the generated projects so no "Trust project?" dialog blocks the run.
    tp = config / "options/trusted-paths.xml"
    root = ET.parse(tp).getroot() if tp.exists() else ET.Element("application")
    comp = root.find("component[@name='Trusted.Paths.Settings']")
    if comp is None:
        comp = ET.SubElement(root, "component", name="Trusted.Paths.Settings")
    opt = comp.find("option[@name='TRUSTED_PATHS']")
    if opt is None:
        opt = ET.SubElement(comp, "option", name="TRUSTED_PATHS")
    lst = opt.find("list")
    if lst is None:
        lst = ET.SubElement(opt, "list")
    ET.SubElement(lst, "option", value=str(work / "projects"))
    tp.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(tp)
    # VM options: the bundle's, the user's heap setting, and our own paths.
    opts = (app / "Contents/bin/pycharm.vmoptions").read_text().splitlines()
    user_opts = user_config / "pycharm.vmoptions"
    if user_opts.exists():
        opts += [l for l in user_opts.read_text().splitlines()
                 if l.startswith("-X")]
    opts += [f"-Didea.config.path={config}", f"-Didea.system.path={prof / 'system'}",
             f"-Didea.log.path={prof / 'log'}", f"-Didea.plugins.path={config / 'plugins'}",
             "-Didea.trust.all.projects=true"]
    vm = prof / "pycharm.vmoptions"
    vm.write_text("\n".join(opts) + "\n")
    return prof, vm


def app_pids(app: Path) -> set:
    out = subprocess.run(["ps", "-A", "-o", "pid=,command="], capture_output=True, text=True).stdout
    return {int(l.split(None, 1)[0]) for l in out.splitlines()
            if l.strip() and str(app) in l}


def tree_rss_mb(pid: int) -> float:
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss="], capture_output=True, text=True).stdout
    rows = [tuple(int(x) for x in l.split()) for l in out.splitlines() if l.strip()]
    children = {}
    for p, pp, _ in rows:
        children.setdefault(pp, []).append(p)
    rss = {p: r for p, _, r in rows}
    total, stack = 0, [pid]
    while stack:
        p = stack.pop()
        total += rss.get(p, 0)
        stack += children.get(p, [])
    return total / 1024


def used_heap_mb(log_dir: Path):
    files = sorted(log_dir.glob("open-telemetry-meters.*.json"))
    if not files:
        return None
    found = []

    def walk(o):
        if isinstance(o, dict):
            if o.get("name") == "JVM.usedHeapBytes":
                for v in re.findall(r'"(?:value|asLong|asDouble|sum)"\s*:\s*([0-9.]+)', json.dumps(o)):
                    found.append(float(v))
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for x in o:
                walk(x)

    try:
        walk(json.loads(files[-1].read_text()))
    except (OSError, ValueError):
        return None
    return round(found[-1] / 1048576) if found else None


def run_once(app: Path, vm: Path, prof: Path, project: Path, idle_s: int, timeout: int):
    log = prof / "log/idea.log"
    offset = log.stat().st_size if log.exists() else 0
    before = app_pids(app)
    env = dict(os.environ, PYCHARM_VM_OPTIONS=str(vm))
    t0 = time.monotonic()
    proc = subprocess.Popen([str(app / "Contents/MacOS/pycharm"), str(project)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    name = project.name
    marks = {}
    buf = ""
    deadline = t0 + timeout
    # The project opens in dumb mode and may re-enter it for indexing: it is
    # indexed once no dumb mode follows the last exit for SETTLE seconds.
    SETTLE = 15
    while time.monotonic() < deadline and "smart" not in marks:
        if "settle" in marks and time.monotonic() - t0 - marks["settle"][0] > SETTLE:
            marks["smart"] = marks["settle"]
        time.sleep(0.2)
        if log.exists():
            with log.open(errors="replace") as fh:
                fh.seek(offset)
                chunk = fh.read()
                offset += len(chunk.encode(errors="replace"))
            buf += chunk
            for line in buf.splitlines():
                m = LOG_TS.match(line)
                if not m:
                    continue
                if f"enter dumb mode [{name}]" in line and "open" not in marks:
                    marks["open"] = (time.monotonic() - t0, int(m.group(1)))
                if "Post-startup activities under progress took" in line and "post" not in marks:
                    marks["post"] = (time.monotonic() - t0, int(m.group(1)))
                if f"enter dumb mode [{name}]" in line:
                    marks["last_enter"] = (time.monotonic() - t0, int(m.group(1)))
                    marks.pop("settle", None)
                if f"exit dumb mode [{name}]" in line:
                    marks.setdefault("ready", (time.monotonic() - t0, int(m.group(1))))
                    marks["settle"] = (time.monotonic() - t0, int(m.group(1)))
            buf = buf[buf.rfind("\n") + 1:] if "\n" in buf else buf
        # A warm start may skip dumb mode entirely: then the end of the
        # post-startup activities is the ready point.
        if "post" in marks and "open" not in marks and time.monotonic() - t0 - marks["post"][0] > 15:
            marks["smart"] = marks["post"]
    keep = ("ready", "smart", "post")
    result = {k: round(v[0] * 1000) for k, v in marks.items() if k in keep}
    result.update({f"{k}_log_ms": v[1] for k, v in marks.items() if k in keep})
    if "smart" not in marks:
        result["error"] = f"no 'exit dumb mode' within {timeout}s"
    elif idle_s:
        time.sleep(idle_s)
        result["rss_mb"] = round(tree_rss_mb(proc.pid))
        result["heap_mb"] = used_heap_mb(prof / "log")
    ours = app_pids(app) - before
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(60)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    time.sleep(3)
    # Helpers such as the embedded browser server may outlive the IDE.
    for pid in ours & app_pids(app):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True, type=Path)
    ap.add_argument("--app", type=Path)
    ap.add_argument("--config", type=Path)
    ap.add_argument("--warm-runs", type=int, default=5)
    ap.add_argument("--idle", type=int, default=120)
    ap.add_argument("--mid-tag", default="3.1.0")
    a = ap.parse_args()
    if running():
        sys.exit("PyCharm is running; close it first")
    app, config = a.app or find_app(), a.config or find_config()
    work = a.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    projects = make_projects(work, a.mid_tag)
    prof, vm = prepare_profile(work, app, config)
    try:
        measure(a, app, prof, vm, projects, work)
    finally:
        shutil.rmtree(prof / "config", ignore_errors=True)  # holds a license copy


def measure(a, app, prof, vm, projects, work):
    version = subprocess.run(["defaults", "read", str(app / "Contents/Info.plist"),
                              "CFBundleShortVersionString"], capture_output=True, text=True).stdout.strip()
    results = {"version": version, "runs": {}}

    def record(key, r):
        results["runs"].setdefault(key, []).append(r)
        print(key, r, flush=True)

    # Small project: first open (cold caches), then warm starts.
    record("small_first", run_once(app, vm, prof, projects["small"], 0, 600))
    for _ in range(a.warm_runs):
        record("small_warm", run_once(app, vm, prof, projects["small"], 0, 300))
    # Mid project: first open indexes it; idle memory; then warm starts.
    record("mid_first", run_once(app, vm, prof, projects["mid"], a.idle, 900))
    for _ in range(max(3, a.warm_runs // 2)):
        record("mid_warm", run_once(app, vm, prof, projects["mid"], 0, 300))

    def med(key, field):
        vals = [r[field] for r in results["runs"].get(key, []) if field in r]
        return (statistics.median(vals), max(vals)) if vals else (None, None)

    summary = {
        "small_first_indexed_ms": med("small_first", "smart")[0],
        "small_warm_ready_ms": med("small_warm", "ready"),
        "mid_first_ready_ms": med("mid_first", "ready")[0],
        "mid_first_indexed_ms": med("mid_first", "smart")[0],
        "mid_warm_ready_ms": med("mid_warm", "ready"),
        "mid_idle_rss_mb": med("mid_first", "rss_mb")[0],
        "mid_idle_heap_mb": med("mid_first", "heap_mb")[0],
    }
    results["summary"] = summary
    (work / "results.json").write_text(json.dumps(results, indent=2))
    print(json.dumps(summary, indent=2))


main()
