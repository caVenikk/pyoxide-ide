#!/usr/bin/env bash
# Tests of scripts/ci-scope.sh with a stub `gh` and a throwaway git
# repository: the draft gate of the CI workflow in the cases that matter,
# including the one of #335 (a run queued while the pull request was a draft,
# started after it was ready).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# gh stub: prints $FAKE_DRAFT, or fails when it is "error".
mkdir "$tmp/bin"
cat >"$tmp/bin/gh" <<'STUB'
#!/usr/bin/env bash
[ "$FAKE_DRAFT" = error ] && exit 1
echo "$FAKE_DRAFT"
STUB
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH" GITHUB_REPOSITORY=owner/repo PULL_REQUEST=7 CI_SCOPE_RETRY_SLEEP=0

# A merge commit like the one the pull_request event checks out: HEAD^1 is
# the base, HEAD the merge.
repo="$tmp/repo"
git init -q "$repo"
git -C "$repo" config user.email t@example.invalid
git -C "$repo" config user.name t
git -C "$repo" config commit.gpgsign false
echo base >"$repo/base.txt"
git -C "$repo" add .
git -C "$repo" commit -qm base
git -C "$repo" checkout -qb topic
mkdir -p "$repo/docs" "$repo/crates"
echo a >"$repo/docs/a.md"
git -C "$repo" add .
git -C "$repo" commit -qm docs
git -C "$repo" checkout -q master 2>/dev/null || git -C "$repo" checkout -q main
git -C "$repo" merge -q --no-ff topic -m merge
docs_only=$(git -C "$repo" rev-parse HEAD)
git -C "$repo" checkout -q -b topic2 topic
echo c >"$repo/crates/lib.rs"
git -C "$repo" add .
git -C "$repo" commit -qm code
git -C "$repo" checkout -q -b code "$(git -C "$repo" rev-parse topic~1)"
git -C "$repo" merge -q --no-ff topic2 -m merge
with_code=$(git -C "$repo" rev-parse HEAD)

failed=0
check() {
    local name=$1 want=$2 got=$3
    if [ "$want" = "$got" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: wanted '$want', got '$got'"
        failed=1
    fi
}

scope() { # event commit draft -> "full draft"
    local out="$tmp/out"
    : >"$out"
    git -C "$repo" checkout -q "$2"
    (cd "$repo" && EVENT=$1 FAKE_DRAFT=$3 GITHUB_OUTPUT=$out bash "$here/ci-scope.sh" scope >/dev/null)
    echo "$(sed -n 's/^full=//p' "$out") $(sed -n 's/^draft=//p' "$out")"
}
recheck() { # draft -> exit status
    (EVENT=pull_request FAKE_DRAFT=$1 bash "$here/ci-scope.sh" recheck >/dev/null 2>&1) && echo 0 || echo 1
}

check "push to main runs everything" "true false" "$(scope push "$with_code" true)"
check "dispatch runs everything" "true false" "$(scope workflow_dispatch "$with_code" true)"
check "draft with code is skipped" "false true" "$(scope pull_request "$with_code" true)"
check "ready with code runs everything" "true false" "$(scope pull_request "$with_code" false)"
check "ready, documentation only, is skipped" "false false" "$(scope pull_request "$docs_only" false)"
check "draft, documentation only, is a draft skip" "false true" "$(scope pull_request "$docs_only" true)"
# The #335 case: the run was queued by the push (payload: draft) and starts
# after `gh pr ready`; the live state wins, whatever event it carries.
check "run queued on a draft, started when ready (#335)" "true false" "$(scope pull_request "$with_code" false)"
check "unreadable API runs everything" "true false" "$(scope pull_request "$with_code" error)"
check "unreadable API, documentation only, still skips docs" "false false" "$(scope pull_request "$docs_only" error)"

check "recheck passes while a draft" 0 "$(recheck true)"
check "recheck fails when ready (stale draft skip)" 1 "$(recheck false)"
check "recheck fails when unreadable" 1 "$(recheck error)"

exit $failed
