#!/usr/bin/env bash
# Gives a fresh checkout or worktree a warm target/: a copy-on-write clone of
# compiled dependencies, so the first `scripts/check.sh` compiles only the
# workspace crates, not the hundreds of dependencies (GPUI, tree-sitter,
# tokio, ...) again. Run it once in a new git worktree, before the first
# build.
#
#   scripts/seed-target.sh              clone the seed into ./target (the seed
#                                       is built or refreshed first when
#                                       Cargo.lock or the toolchain changed)
#   scripts/seed-target.sh --refresh    only build or refresh the seed
#   scripts/seed-target.sh --from DIR   clone DIR (the target/ of another
#                                       checkout, say the main one) instead
#                                       of the seed; nothing is built
#
# The seed is $PYOXIDE_TARGET_SEED, default ~/.cache/pyoxide/target-seed. It
# is built with the same cargo commands and flags as scripts/check.sh (host
# and, where installed, the Windows target), then the workspace crates' own
# artifacts are removed: a checkout compiles them in its own path anyway, and
# the clone stays small. Nothing builds in a checkout's target/ for it.
#
# What is cloned: the dev profile of the host and of the Windows target
# (target/debug, target/x86_64-pc-windows-gnu/debug), without incremental
# state. Release builds, nextest output and timings are not.
#
# Limits:
# - The clone costs no disk space until files change: APFS clonefile on
#   macOS, reflink on Linux file systems that support it (btrfs, xfs). On any
#   other file system it is skipped (set PYOXIDE_SEED_COPY=1 to copy the
#   files instead; a seed is gigabytes).
# - With --from, the clone takes the cargo build locks of DIR (shared) while
#   it copies, so a cargo build running there waits for the clone instead of
#   being read half-written, and a clone waits for a running build (up to
#   PYOXIDE_SEED_WAIT seconds, default 600, then it is skipped). Test
#   binaries running from DIR do not matter: they write nothing there.
#   Without the locks (a cargo that uses none) --from is not safe against a
#   concurrent build.
# - A checkout whose target/ is not empty is left alone.
# - A different Cargo.lock than the seed's only costs the compile of the
#   dependencies that differ; a different toolchain makes the clone useless
#   (cargo rebuilds everything), so the seed is rebuilt for it first.
set -euo pipefail
cd "$(dirname "$0")/.."

seed=${PYOXIDE_TARGET_SEED:-$HOME/.cache/pyoxide/target-seed}
WINDOWS_TARGET=x86_64-pc-windows-gnu
refresh_only=0
from=""
while [ $# -gt 0 ]; do
    case "$1" in
        --refresh) refresh_only=1 ;;
        --from)
            from=${2:?seed-target.sh: --from needs a directory}
            shift
            ;;
        -h | --help)
            sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "seed-target.sh: unknown argument '$1' (see --help)" >&2
            exit 2
            ;;
    esac
    shift
done

build_seed() {
    local stamp
    stamp="$(shasum <Cargo.lock | cut -d" " -f1) $(rustc -V)"
    if [ "$(cat "$seed/.seed-stamp" 2>/dev/null || true)" = "$stamp" ]; then
        return
    fi
    echo "seed-target: building the seed in $seed (Cargo.lock or the toolchain changed)"
    mkdir -p "$seed"
    (
        export CARGO_TARGET_DIR=$seed
        # The dependencies of every workspace crate, for lints and for tests,
        # as check.sh compiles them.
        cargo nextest run --workspace --profile ci --no-run
        cargo clippy --workspace --all-targets
        if rustup target list --installed 2>/dev/null | grep -qx "$WINDOWS_TARGET" \
            && command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
            cargo clippy --workspace --all-targets --target "$WINDOWS_TARGET"
        fi
        # Only the dependencies stay.
        members=$(cargo metadata --format-version 1 --no-deps \
            | python3 -c 'import json,sys; print(" ".join("-p " + p["name"] for p in json.load(sys.stdin)["packages"]))')
        # shellcheck disable=SC2086
        cargo clean $members
        # shellcheck disable=SC2086
        cargo clean $members --target "$WINDOWS_TARGET" 2>/dev/null || true
    )
    rm -rf "$seed/debug/incremental" "$seed/$WINDOWS_TARGET/debug/incremental"
    echo "$stamp" >"$seed/.seed-stamp"
}

# One clone of the file tree $1 into $2, keeping modification times (cargo
# compares them). Fails when the file system cannot clone.
clone_tree() {
    case "$(uname -s)" in
        Darwin) cp -cpR "$1" "$2" ;;
        Linux)
            if [ "${PYOXIDE_SEED_COPY:-0}" = 1 ]; then
                cp -a --reflink=auto "$1" "$2"
            else
                cp -a --reflink=always "$1" "$2"
            fi
            ;;
        *)
            [ "${PYOXIDE_SEED_COPY:-0}" = 1 ] || return 1
            cp -a "$1" "$2"
            ;;
    esac
}

# The profile directories of $1 (a target directory) into ./target, without
# incremental state.
clone_profiles() {
    local src=$1 profile child
    mkdir -p target
    for profile in debug "$WINDOWS_TARGET/debug"; do
        [ -d "$src/$profile" ] || continue
        mkdir -p "target/$profile"
        for child in "$src/$profile"/* "$src/$profile"/.[!.]*; do
            [ -e "$child" ] || continue
            [ "$(basename "$child")" = incremental ] && continue
            clone_tree "$child" "target/$profile/" || return 1
        done
    done
    for child in CACHEDIR.TAG .rustc_info.json; do
        if [ -f "$src/$child" ]; then cp -p "$src/$child" target/; fi
    done
    return 0
}

# Runs "$@" while holding shared locks on cargo's lock files in $1/debug (and
# the Windows profile), the files a cargo build locks exclusively.
with_build_locks() {
    local dir=$1
    shift
    local files=()
    local f
    for f in "$dir"/debug/.cargo-lock "$dir"/debug/.cargo-build-lock "$dir"/debug/.cargo-artifact-lock \
        "$dir/$WINDOWS_TARGET"/debug/.cargo-lock "$dir/$WINDOWS_TARGET"/debug/.cargo-build-lock \
        "$dir/$WINDOWS_TARGET"/debug/.cargo-artifact-lock; do
        if [ -e "$f" ]; then files+=("$f"); fi
    done
    python3 - "${PYOXIDE_SEED_WAIT:-600}" "${files[@]}" -- "$@" <<'PY'
import fcntl, subprocess, sys, time

wait = float(sys.argv[1])
sep = sys.argv.index("--")
files, cmd = sys.argv[2:sep], sys.argv[sep + 1:]
held = []
deadline = time.monotonic() + wait
waited = False
for path in files:
    fd = open(path, "rb")
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
            break
        except OSError:
            if not waited:
                print("seed-target: a cargo build is running in the source, waiting for it", file=sys.stderr)
                waited = True
            if time.monotonic() > deadline:
                print("seed-target: the source stayed busy, skipped", file=sys.stderr)
                sys.exit(75)
            time.sleep(2)
    held.append(fd)
sys.exit(subprocess.call(cmd))
PY
}

if [ -z "$from" ]; then
    build_seed
    src=$seed
else
    src=$from
    [ -d "$src/debug" ] || {
        echo "seed-target: $src has no debug/ directory" >&2
        exit 2
    }
fi

if [ "$refresh_only" -eq 1 ]; then
    echo "seed-target: seed is current ($seed)"
    exit 0
fi

if [ -d target ] && [ -n "$(ls -A target 2>/dev/null)" ]; then
    echo "seed-target: target/ is not empty, left alone"
    exit 0
fi

start=$(date +%s)
status=0
if [ -n "$from" ]; then
    with_build_locks "$src" bash -c "$(declare -f clone_profiles clone_tree); WINDOWS_TARGET=$WINDOWS_TARGET; clone_profiles '$src'" || status=$?
else
    clone_profiles "$src" || status=$?
fi
if [ "$status" -ne 0 ]; then
    rm -rf target
    echo "seed-target: skipped (no copy-on-write file system, or the source was busy); the first build compiles everything" >&2
    exit 0
fi
echo "seed-target: target/ cloned from $src in $(($(date +%s) - start)) s"
