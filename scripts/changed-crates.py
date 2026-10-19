#!/usr/bin/env python3
# The workspace crates a change can affect, for `scripts/check.sh --quick`:
# the crates whose files changed against a base (committed, staged, unstaged
# or untracked) and every workspace crate that depends on them, directly or
# not, through any kind of dependency (normal, build or dev).
#
#     python3 scripts/changed-crates.py [--lib] [BASE]   # BASE defaults to origin/main
#
# --lib: only the crates with a library target (for doctests).
#
# Prints one crate name per line; `*` when a change can affect every crate
# (Cargo.lock, the root Cargo.toml, .cargo/, .config/, a test fixture, any
# file it does not know); nothing when only files outside the crates
# changed (documentation, scripts, notices, workflows). Standard library only.
import json
import subprocess
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent

# Files outside crates/ that no build or test reads.
NO_CRATE_DIRS = ("docs/", "replica/", "skills/", "notices/", "scripts/", ".github/", "tests/scenarios/")
NO_CRATE_FILES = ("LICENSE-APACHE", "LICENSE-MIT", "NOTICE", "docker-compose.yml", ".gitignore", ".gitattributes")


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
    args = [a for a in args if a != "--lib"]
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
