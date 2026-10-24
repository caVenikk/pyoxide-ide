#!/usr/bin/env bash
# Gives a fresh checkout or worktree a warm target/: a copy-on-write clone of
# the compiled dependencies kept in a seed directory, so the first
# `scripts/check.sh` compiles only the workspace crates, not the hundreds of
# dependencies (GPUI, tree-sitter, tokio, ...) again.
#
#   scripts/seed-target.sh            clone the seed into ./target (builds or
#                                     refreshes the seed first when Cargo.lock
#                                     changed since it was built)
#   scripts/seed-target.sh --refresh  only build or refresh the seed
#
# The seed is $PYOXIDE_TARGET_SEED, default ~/.cache/pyoxide/target-seed. It is
# built with the same commands and flags as scripts/check.sh (host and, where
# installed, the Windows target), then the workspace crates' own artifacts are
# removed: they would be rebuilt in the new path anyway. The clone costs no
# disk space until files change (APFS clonefile on macOS, reflink on Linux
# file systems that support it; elsewhere a plain copy). A checkout whose
# target/ is not empty is left alone.
set -euo pipefail
cd "$(dirname "$0")/.."

seed=${PYOXIDE_TARGET_SEED:-$HOME/.cache/pyoxide/target-seed}
WINDOWS_TARGET=x86_64-pc-windows-gnu
refresh_only=0
case "${1:-}" in
    --refresh) refresh_only=1 ;;
    "") ;;
    *)
        echo "seed-target.sh: unknown argument '$1'" >&2
        exit 2
        ;;
esac

lock_hash=$(shasum < Cargo.lock | cut -d" " -f1)
rustc_version=$(rustc -V)
stamp="$lock_hash $rustc_version"

if [ "$(cat "$seed/.seed-stamp" 2>/dev/null || true)" != "$stamp" ]; then
    echo "seed-target: building the seed in $seed (Cargo.lock or the toolchain changed)"
    mkdir -p "$seed"
    export CARGO_TARGET_DIR=$seed
    # Dependencies of every workspace crate, for tests and lints, as check.sh
    # compiles them.
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
    cargo clean $members --target "$WINDOWS_TARGET" 2>/dev/null || true
    rm -rf "$seed/debug/incremental" "$seed/$WINDOWS_TARGET/debug/incremental"
    echo "$stamp" >"$seed/.seed-stamp"
    unset CARGO_TARGET_DIR
fi

if [ "$refresh_only" -eq 1 ]; then
    echo "seed-target: seed is current ($seed)"
    exit 0
fi

if [ -d target ] && [ -n "$(ls -A target 2>/dev/null)" ]; then
    echo "seed-target: target/ is not empty, left alone"
    exit 0
fi
rm -rf target
case "$(uname -s)" in
    Darwin) cp -cpR "$seed" target ;;
    Linux) cp -a --reflink=auto "$seed" target ;;
    *) cp -a "$seed" target ;;
esac
rm -f target/.seed-stamp
echo "seed-target: target/ cloned from $seed"
