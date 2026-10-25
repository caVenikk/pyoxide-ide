#!/usr/bin/env bash
# The scope decisions of the CI workflow (.github/workflows/ci.yml, job
# `scope`), in a script so that scripts/test-ci-scope.sh can test them.
#
#   scripts/ci-scope.sh scope     writes full=true|false and draft=true|false
#                                 to $GITHUB_OUTPUT (stdout when unset)
#   scripts/ci-scope.sh recheck   for the jobs that skipped because the pull
#                                 request was a draft: fails when it is not
#                                 a draft any more
#
# Environment: EVENT (github.event_name), PULL_REQUEST (the number),
# GITHUB_REPOSITORY, and gh with GH_TOKEN. `scope` runs in a checkout of the
# pull request's merge commit (two commits deep: HEAD^1 is the base branch).
#
# The draft state is read from the API when the script runs, never from the
# event payload: the payload carries the state of the moment the run was
# queued. A push to a draft and `gh pr ready` a second apart queue two runs of
# the same commit, and the push's says draft=true. The newer run cancels the
# older (concurrency in ci.yml), but when the survivor is the stale one, its
# jobs pass in seconds without testing anything while the pull request is
# already ready, and it merges on those green checks (#335). Read live, every
# run that starts after the pull request became ready does the full run,
# whichever event started it. When the API cannot be read, everything runs:
# it is the skip that has to be certain.
set -euo pipefail

out=${GITHUB_OUTPUT:-/dev/stdout}

# true, false or unknown.
live_draft() {
    local draft attempt
    for attempt in 1 2; do
        if draft=$(gh api "repos/$GITHUB_REPOSITORY/pulls/$PULL_REQUEST" --jq .draft 2>/dev/null) \
            && { [ "$draft" = true ] || [ "$draft" = false ]; }; then
            echo "$draft"
            return
        fi
        sleep "${CI_SCOPE_RETRY_SLEEP:-3}"
    done
    echo unknown
}

case "${1:-}" in
    scope)
        full=true
        draft=false
        if [ "${EVENT:-}" = pull_request ]; then
            state=$(live_draft)
            if [ "$state" = true ]; then
                full=false
                draft=true
                echo "Draft pull request: the OS matrix runs when it is ready for review."
            elif ! git diff --name-only HEAD^1 HEAD \
                | grep -qvE '(\.md$|^docs/|^replica/|^tests/scenarios/)'; then
                full=false
                echo "Only documentation changed: nothing to build or test."
            fi
        fi
        {
            echo "full=$full"
            echo "draft=$draft"
        } >>"$out"
        ;;
    recheck)
        state=$(live_draft)
        if [ "$state" = true ]; then
            echo "Still a draft: skipping is right."
        else
            echo "::error::The pull request is not a draft any more ($state), so this skipped run must not count as a pass. The run for the ready_for_review event does the full check; re-run it if it is missing."
            exit 1
        fi
        ;;
    *)
        echo "ci-scope.sh: expected 'scope' or 'recheck'" >&2
        exit 2
        ;;
esac
