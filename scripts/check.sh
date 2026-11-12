#!/usr/bin/env bash
# The checks of CI (.github/workflows/ci.yml) on this machine. CI calls this
# script for each of its steps, so the commands and flags are the same in
# both places. Before a task push use --quick --base <actual-target>.
#
#   scripts/check.sh              every step, the whole workspace
#   scripts/check.sh --quick      only the crates changed against origin/main
#                                 (committed or not) and the crates that
#                                 depend on them; CI checks the rest
#   scripts/check.sh --quick --base REF  compare with the exact target tree
#   scripts/check.sh --plan FILE   use the exact JSON scope produced by ci-plan.py
#   scripts/check.sh STEP...      only these steps (with or without --quick)
#
# Steps, in this order:
#   fmt             cargo fmt --check
#   clippy          clippy for this machine's target
#   clippy-windows  clippy for x86_64-pc-windows-gnu: catches what fails only
#                   on Windows (items used only under cfg(unix)). Needs the
#                   rustup target and mingw-w64; skipped with a note when
#                   they are missing and on Windows. CI has a Windows runner
#                   instead. With --quick uses the full affected closure.
#   build           the test binaries (cargo nextest run --no-run)
#   test            the tests (cargo nextest run, profile `ci`)
#   throughput      terminal throughput (ADR 0014), numbers to the log
#   doc             doctests, which nextest does not run
#   repo            progress, layer, brand and notice checks, the tests of the CI
#                   scope script (job `progress`)
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
base=origin/main
plan=""
steps=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        --quick) quick=1 ;;
        --base | --plan)
            if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                echo "check.sh: $1 requires a value" >&2
                exit 2
            fi
            if [ "$1" = --base ]; then base="$2"; else plan="$2"; fi
            shift
            ;;
        -h | --help)
            sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
            exit 0
            ;;
        fmt | clippy | clippy-windows | build | test | throughput | doc | repo) steps+=("$1") ;;
        *) echo "check.sh: unknown argument '$1' (see --help)" >&2; exit 2 ;;
    esac
    shift
done
if [ -n "$plan" ] && [ "$quick" -eq 1 ]; then
    echo "check.sh: --plan and --quick are mutually exclusive" >&2
    exit 2
fi
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
win_scope=(--workspace)
doc_scope=(--workspace)
crates=()
if [ -n "$plan" ] || [ "$quick" -eq 1 ]; then
    if [ -n "$plan" ]; then
        plan_json=$(cat "$plan")
    else
        plan_json=$("$PYTHON" scripts/ci-plan.py --base "$base" --base-ref integration/local)
    fi
    # Validate before using any names in cargo. Never silently accept an empty
    # or malformed plan; CI owns the plan and local quick uses the same parser.
    parsed=$(printf '%s' "$plan_json" | "$PYTHON" -I -c '
import json, re, sys
p = json.load(sys.stdin)
mode = p["mode"]
assert mode in {"full", "fast", "docs"}, "unsupported plan mode"
for key in ("packages", "libs"):
    names = p[key]
    assert names == "*" or (isinstance(names, list) and all(isinstance(n, str) and re.fullmatch(r"[A-Za-z0-9_-]+", n) for n in names)), "invalid packages"
    assert (mode == "full" and names == "*") or (mode != "full" and isinstance(names, list)), "inconsistent scope"
    assert mode != "docs" or not names, "docs plan contains packages"
assert mode != "fast" or p["packages"], "empty fast plan"
assert p["libs"] == "*" or set(p["libs"]) <= set(p["packages"]), "libraries outside scope"
print(" ".join(p["packages"]) if p["packages"] != "*" else "*")
print(" ".join(p["libs"]) if p["libs"] != "*" else "*")
')
    affected=$(printf '%s\n' "$parsed" | sed -n '1p')
    libs=$(printf '%s\n' "$parsed" | sed -n '2p')
    if [ "$affected" != "*" ]; then
        scope=()
        doc_scope=()
        for c in $affected; do crates+=("$c"); scope+=(-p "$c"); done
        for c in $libs; do doc_scope+=(-p "$c"); done
        win_scope=("${scope[@]}")
        echo "check.sh: affected packages: ${crates[*]-none}"
    else
        echo "check.sh: full workspace validation"
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
            if [ ${#win_scope[@]} -gt 0 ] && windows_ready; then
                cargo clippy "${win_scope[@]}" --all-targets --target "$WINDOWS_TARGET" -- -D warnings
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
            "$PYTHON" scripts/check-brand.py
            "$PYTHON" scripts/notice.py --check
            bash scripts/test-ci-scope.sh
            "$PYTHON" -m unittest discover -s scripts -p 'test_ci_plan.py'
            ;;
    esac
}

# The time of each step, printed at the end (also when a step fails, with the
# failing one marked) so the tail of a log shows where the time went and what
# stopped the run.
timings=()
current=""
current_start=0
script_start=$(date +%s)

summary() {
    local rc=$?
    local now
    now=$(date +%s)
    if [ -n "$current" ]; then
        timings+=("$current $((now - current_start)) FAILED")
    fi
    if [ ${#timings[@]} -gt 0 ]; then
        echo
        echo "== check.sh: $((now - script_start)) s"
        local entry name secs mark
        for entry in "${timings[@]}"; do
            read -r name secs mark <<<"$entry"
            printf '%-15s %5s s  %s\n' "$name" "$secs" "$mark"
        done
    fi
    if [ "$rc" -ne 0 ]; then
        echo "check.sh: FAILED${current:+ at step $current}" >&2
    fi
}
trap summary EXIT

for step in "${steps[@]}"; do
    echo "==> $step"
    current=$step
    current_start=$(date +%s)
    run_step "$step"
    elapsed=$(($(date +%s) - current_start))
    echo "<== $step: $elapsed s"
    timings+=("$step $elapsed")
    current=""
done
