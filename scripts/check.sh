#!/usr/bin/env bash
# The checks of CI (.github/workflows/ci.yml) on this machine. CI calls this
# script for each of its steps, so the commands and flags are the same in
# both places. Run it before every push.
#
#   scripts/check.sh              every step, the whole workspace
#   scripts/check.sh --quick      only the crates changed against origin/main
#                                 (committed or not) and the crates that
#                                 depend on them; CI checks the rest
#   scripts/check.sh STEP...      only these steps (with or without --quick)
#
# Steps, in this order:
#   fmt             cargo fmt --check
#   clippy          clippy for this machine's target
#   clippy-windows  clippy for x86_64-pc-windows-gnu: catches what fails only
#                   on Windows (items used only under cfg(unix)). Needs the
#                   rustup target and mingw-w64; skipped with a note when
#                   they are missing and on Windows. CI has a Windows runner
#                   instead.
#   build           the test binaries (cargo nextest run --no-run)
#   test            the tests (cargo nextest run, profile `ci`)
#   throughput      terminal throughput (ADR 0014), numbers to the log
#   doc             doctests, which nextest does not run
#   repo            progress, layer and notice checks (job `progress`)
#
# Needs cargo-nextest (`cargo install cargo-nextest --locked`) and python3.
set -euo pipefail
cd "$(dirname "$0")/.."

# The cargo commands use the workspace's dev profile as it is, the same as
# a local debug build, so a check reuses what `cargo build` and `cargo test`
# compiled (ADR 0007). CI only adds settings that change no result
# (.github/workflows/ci.yml, job check).

ALL_STEPS=(fmt clippy clippy-windows build test throughput doc repo)
WINDOWS_TARGET=x86_64-pc-windows-gnu

quick=0
steps=()
for arg in "$@"; do
    case "$arg" in
        --quick) quick=1 ;;
        -h | --help)
            sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
            exit 0
            ;;
        fmt | clippy | clippy-windows | build | test | throughput | doc | repo) steps+=("$arg") ;;
        *)
            echo "check.sh: unknown argument '$arg' (see --help)" >&2
            exit 2
            ;;
    esac
done
if [ ${#steps[@]} -eq 0 ]; then
    steps=("${ALL_STEPS[@]}")
fi

# python3, or python where that is the name (Windows); a python3 that does
# not run (the Microsoft Store alias) does not count.
if python3 -c '' >/dev/null 2>&1; then
    PYTHON=python3
else
    PYTHON=python
fi

# The packages the cargo steps check: the workspace, or with --quick the
# changed crates and their dependents.
scope=(--workspace)
doc_scope=(--workspace)
crates=()
if [ "$quick" -eq 1 ]; then
    affected=$("$PYTHON" scripts/changed-crates.py)
    if [ "$affected" = "*" ]; then
        echo "check.sh --quick: a change can affect every crate, checking the workspace"
    else
        scope=()
        for c in $affected; do
            crates+=("$c")
            scope+=(-p "$c")
        done
        # Doctests only for crates with a library.
        doc_scope=()
        libs=$("$PYTHON" scripts/changed-crates.py --lib)
        for c in $libs; do
            doc_scope+=(-p "$c")
        done
        if [ ${#crates[@]} -eq 0 ]; then
            echo "check.sh --quick: no crate changed against origin/main"
        else
            echo "check.sh --quick: ${crates[*]}"
        fi
    fi
fi

# Whether a cargo step has anything to check.
has_scope() {
    [ ${#scope[@]} -gt 0 ]
}

# Whether the checked packages include crate $1.
includes() {
    [ "${scope[0]:-}" = --workspace ] || [[ " ${crates[*]-} " == *" $1 "* ]]
}

windows_ready() {
    case "$(uname -s)" in
        MINGW* | MSYS* | CYGWIN*)
            echo "on Windows step clippy covers it"
            return 1
            ;;
    esac
    if ! rustup target list --installed 2>/dev/null | grep -qx "$WINDOWS_TARGET"; then
        echo "skipped: run 'rustup target add $WINDOWS_TARGET'"
        return 1
    fi
    if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
        echo "skipped: install mingw-w64 (brew install mingw-w64, apt install mingw-w64)"
        return 1
    fi
}

# Steps are plain calls, so `set -e` stops the script at the first failing
# command; a step with nothing to check does nothing.
run_step() {
    case "$1" in
        fmt) cargo fmt --all --check ;;
        clippy)
            if has_scope; then
                cargo clippy "${scope[@]}" --all-targets -- -D warnings
            fi
            ;;
        clippy-windows)
            if has_scope && windows_ready; then
                cargo clippy "${scope[@]}" --all-targets --target "$WINDOWS_TARGET" -- -D warnings
            fi
            ;;
        build)
            if has_scope; then
                cargo nextest run "${scope[@]}" --profile ci --no-run --timings
            fi
            ;;
        test)
            if has_scope; then
                cargo nextest run "${scope[@]}" --profile ci
            fi
            ;;
        throughput)
            if includes terminal; then
                cargo test -p terminal --test it throughput:: -- --ignored --nocapture
            fi
            ;;
        doc)
            if [ ${#doc_scope[@]} -gt 0 ]; then
                cargo test --doc "${doc_scope[@]}"
            fi
            ;;
        repo)
            "$PYTHON" scripts/progress.py --check-data
            "$PYTHON" scripts/check-layers.py
            "$PYTHON" scripts/notice.py --check
            ;;
    esac
}

for step in "${steps[@]}"; do
    echo "==> $step"
    start=$(date +%s)
    run_step "$step"
    echo "<== $step: $(($(date +%s) - start)) s"
done
