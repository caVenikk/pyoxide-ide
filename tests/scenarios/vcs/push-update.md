# Push and Update Project

## Steps
1. Use a local bare repository as `origin`. Commit on `main`; look at
   the widget; press Cmd+Shift+K (macOS) or Ctrl+Shift+K; press Push.
2. Create a branch `feat/x`, commit, push it.
3. Make `origin/main` move ahead from another clone; commit locally and
   push.
4. In the "Push Rejected" dialog choose Merge.
5. Make `origin/main` move ahead again; press Cmd+T / Ctrl+T; choose
   merge and press OK.
6. Run Update Project on a branch without an upstream.

## Expected
- Step 1: an outgoing arrow on the widget; the dialog "Push Commits to
  <repo>" lists `main → origin : main` and the commit; a balloon
  "Pushed 1 commit to origin/main".
- Step 2: the target shows a "New" tag; after the push the upstream is
  set.
- Step 3: the dialog "Push Rejected" with Merge, Rebase and Cancel.
- Step 4: the remote changes are merged and the push succeeds.
- Step 5: the dialog "Update Project" (merge or rebase, stash or shelve,
  "Don't show again"); local changes are stashed and restored; a balloon
  "1 file updated in 1 commit"; the incoming arrow goes away.
- Step 6: an error that the branch has no tracked branch.

## Commands
`vcs.push`, `vcs.update-project {"method": ...}`, `git.fetch`,
`vcs.state` (widget arrows), `workbench.state` (notifications).

## Source
https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html,
https://www.jetbrains.com/help/pycharm/sync-with-a-remote-repository.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/git4idea`
(push, update), `GitBundle`. Dialog layouts are to verify against a
reference screenshot.
