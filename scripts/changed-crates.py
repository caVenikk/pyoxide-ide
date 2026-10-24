#!/usr/bin/env python3
# The workspace crates a change can affect, for `scripts/check.sh --quick`:
# the crates whose files changed against a base (committed, staged, unstaged
# or untracked) and every workspace crate that depends on them, directly or
# not, through any kind of dependency (normal, build or dev).
#
#     python3 scripts/changed-crates.py [--lib] [BASE]   # BASE defaults to origin/main
#
# --lib: only the crates with a library target (for doctests).
# --windows: only the crates that can fail on Windows alone (for the Windows
# clippy step): the changed crates with platform-specific code (a cfg on the
# OS, `std::os::unix`, `libc`, ...), and the direct dependents of a changed
# crate when either of the two has such code. A crate without platform code
# compiles the same source on every OS, so the host's clippy has checked it.
#
# Prints one crate name per line; `*` when a change can affect every crate
# (Cargo.lock, the root Cargo.toml, .cargo/, .config/, a test fixture, any
# file it does not know); nothing when only files outside the crates
# changed (documentation, scripts, notices, workflows). Standard library only.
import json
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent

# Files outside crates/ that no build or test reads.
NO_CRATE_DIRS = ("docs/", "replica/", "skills/", "notices/", "scripts/", ".github/", "tests/scenarios/")
NO_CRATE_FILES = ("LICENSE-APACHE", "LICENSE-MIT", "NOTICE", "docker-compose.yml", ".gitignore", ".gitattributes")


# Source that is compiled differently, or not at all, on some OS: a cfg or
# cfg! naming the OS, the OS-specific parts of std, and the crates that wrap
# an OS API.
PLATFORM = re.compile(
    r"\bcfg(_attr)?!?\s*\(.*\b(unix|windows|target_os|target_family|target_env|target_vendor)\b"
    r"|\bos::(unix|windows)\b"
    r"|\b(libc|nix|winapi|windows|windows_sys)::"
)


def has_platform_code(crate_dir: Path) -> bool:
    for f in crate_dir.rglob("*.rs"):
        if "target" in f.relative_to(crate_dir).parts[:1]:
            continue
        if PLATFORM.search(f.read_text(encoding="utf-8", errors="replace")):
            return True
    return False


def git(*args: str) -> list[str]:
    out = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return [line for line in out.splitlines() if line]


def changed_files(base: str) -> list[str]:
    merge_base = git("merge-base", base, "HEAD")[0]
    # Against the working tree: committed, staged and unstaged changes; both
    # sides of a rename.
    files = git("diff", "--name-only", "--no-renames", merge_base)
    return files + git("ls-files", "--others", "--exclude-standard")


def main() -> int:
    args = sys.argv[1:]
    lib_only = "--lib" in args
    windows = "--windows" in args
    args = [a for a in args if a not in ("--lib", "--windows")]
    base = args[0] if args else "origin/main"
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"], cwd=ROOT, check=True, capture_output=True, text=True
        ).stdout
    )
    members = set(meta["workspace_members"])
    names = {p["id"]: p["name"] for p in meta["packages"] if p["id"] in members}
    dirs = {
        p["id"]: PurePosixPath(Path(p["manifest_path"]).parent.relative_to(ROOT).as_posix())
        for p in meta["packages"]
        if p["id"] in members
    }

    changed: set[str] = set()
    for f in changed_files(base):
        path = PurePosixPath(f)
        owners = [pid for pid, d in dirs.items() if d in path.parents]
        if owners:
            # The innermost crate directory owns the file.
            changed.add(max(owners, key=lambda pid: len(dirs[pid].parts)))
        elif f.endswith(".md") or f.startswith(NO_CRATE_DIRS) or f in NO_CRATE_FILES:
            continue
        else:
            print("*")
            return 0

    dependents: dict[str, set[str]] = {pid: set() for pid in members}
    for node in meta["resolve"]["nodes"]:
        if node["id"] not in members:
            continue
        for dep in node["deps"]:
            if dep["pkg"] in members:
                dependents[dep["pkg"]].add(node["id"])

    affected = set()
    todo = list(changed)
    while todo:
        pid = todo.pop()
        if pid not in affected:
            affected.add(pid)
            todo.extend(dependents[pid])
    if windows:
        platform = {pid for pid in members if has_platform_code(ROOT / dirs[pid])}
        # What can fail on Windows only: the changed crates with platform
        # code, and the direct dependents of a changed crate when either
        # side has platform code (a `cfg(windows)` item of the one that the
        # other uses without a cfg, or the reverse). A dependent further
        # away sees the change only through the crates in between, which
        # the host's build has compiled the same way.
        affected = {pid for pid in changed if pid in platform}
        for pid in changed:
            for dep in dependents[pid]:
                if pid in platform or dep in platform:
                    affected.add(dep)
    libs = {
        p["id"]
        for p in meta["packages"]
        if p["id"] in members and any("lib" in k or k == "proc-macro" for t in p["targets"] for k in t["kind"])
    }
    for name in sorted(names[pid] for pid in affected if not lib_only or pid in libs):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
