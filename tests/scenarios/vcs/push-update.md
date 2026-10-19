# Push and Update Project

## Steps
1. Use a local bare repository as `origin` (`demo-remote.git` beside the
   project), `main` tracking `origin/main`. Commit on `main`; look at the
   widget; press Cmd+Shift+K (macOS) or Ctrl+Shift+K; select the commit;
   press Push.
2. Create a branch `feat/x`, commit, press Cmd+Shift+K, push it.
3. Amend the commit of `feat/x`; open the Push dialog; choose Force Push
   from the arrow of Push; confirm.
4. Make `origin/main` move ahead from another clone; commit locally and
   push.
5. In the "Push Rejected" dialog choose Merge.
6. Repeat step 4, choose Cancel.
7. Make `origin/main` move ahead again; change a file locally without
   committing; press Cmd+T / Ctrl+T; choose Rebase, tick "Don't show
   again", press OK. Press Cmd+T again.
8. Change the first line of a file on the remote and differently in a
   local commit; run Update Project with Merge.
9. Run Update Project on a branch without an upstream.
10. Open the branches popup: press Fetch (the arrow beside the search);
    open the submenu of `origin/main`; open the submenu of a local branch
    that is not checked out and choose Update.
11. Press Ctrl+V (macOS) or Alt+` (VCS Operations).

## Expected
- Step 1: an outgoing arrow on the widget; the dialog "Push Commits to
  demo" lists `main → origin : main` and the commits to push, the files
  of the selection on the right ("No commits selected" without one);
  Push tags with All / Current Branch at the bottom left; a balloon
  "Pushed 1 commit to origin/main"; the arrow goes away.
- Step 2: the target shows a `New` label; the balloon "Pushed feat/x to
  new branch origin/feat/x"; the upstream is set. A click on the target
  branch edits it in place; for a target other than the upstream "Set
  upstream" shows beside it.
- Step 3: "Force Push" asks "You're going to force push to 'feat/x'. It
  may overwrite commits at the remote. Are you sure you want to proceed?"
  with "Don't warn about this target"; Cancel returns to the Push dialog;
  Force Push pushes with `--force-with-lease`: "Force pushed feat/x to
  origin/feat/x". For `main` and `master` Force Push is disabled ("Force
  push to main is prohibited").
- Step 4: the dialog "Push Rejected": "Push of the current branch "main"
  was rejected. Remote changes need to be merged before pushing." with
  Merge (default), Rebase and Cancel and "Remember the update method and
  update silently in the future".
- Step 5: the remote changes are merged and the push succeeds: "Pushed 2
  commits to origin/main" with "1 commit received during the push
  operation".
- Step 6: a warning "Push rejected": "Push to origin/main was rejected".
- Step 7: the dialog "Update Project" (Merge incoming changes into the
  current branch / Rebase the current branch on top of incoming changes,
  "Don't show again"); the local change is stashed and restored; a
  balloon "1 file updated in 1 commit"; the incoming arrow goes away. The
  second Cmd+T runs without the dialog: "All files are up to date".
- Step 8: a warning "Cannot complete update": "There are pending
  unresolved conflicts."; the repository stays merging.
- Step 9: an error "Cannot update": "<branch> has no tracked branch".
- Step 10: "Fetch successful" (with "Pruned obsolete remote reference:
  origin/…" when a remote branch is gone); the submenu of `origin/main`
  offers Pull into 'main' Using Rebase and Using Merge; Update of another
  branch fast-forwards it: "Fetched 1 targets".
- Step 11: the popup "VCS Operations" in the middle of the window: Git,
  Commit…, Rollback…, Show History, Annotate with Git Blame, Compare with
  the Same Repository Version, Branches…, Push…, Stash Changes…, Unstash
  Changes…, Add to VCS, Copy Branch Name, Resolve Conflicts…; those not
  there yet are disabled.

## Limitations
- git never asks for credentials: remotes need a credential helper or an
  SSH agent; an authentication failure is a "Push failed" or "Fetch
  failed" balloon with git's message (credentials removed).
- What the dialogs remember (the update method, Don't show again, the
  Push Rejected choice, targets force pushed without a warning) lasts
  while the IDE runs.

## Commands
`vcs.push {"dialog": false}`, `vcs.update-project {"method": ...}`,
`git.fetch`, `git.pull`, `git.update-branch`, `vcs.operations-popup`,
`push-dialog.*`, `update-dialog.*`, `dialog.choose`, `vcs.state`
(`remote`: the dialogs; the widget's arrows), `workbench.state`
(notifications). Tests: `crates/platform/workbench/tests/it/remote.rs`,
`crates/platform/git/tests/it/remote.rs`.

## Source
https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html,
https://www.jetbrains.com/help/pycharm/sync-with-a-remote-repository.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/git4idea`
(push, update, fetch), `platform/dvcs-impl` (the Push dialog, Force Push),
`GitBundle`, `DvcsBundle`. The Push and Update Project dialogs are to
verify against a reference screenshot.
