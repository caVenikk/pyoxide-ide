# Commit edge cases

## Steps
1. In the Commit tool window untick every file and press Commit.
2. Tick a file, leave the message empty, press Commit.
3. Tick only an unversioned file, type a message, Commit.
4. Unset `user.name` and `user.email` for the repository and globally;
   commit.
5. Add a pre-commit hook that fails; commit. Open the gear (Ctrl+O),
   untick Run Git hooks, commit again.
6. Check out a commit (detached HEAD) and commit.

## Expected
- Step 1: a red line "Select files to commit"; nothing is committed.
- Step 2: a red line "Specify commit message".
- Step 3: the file is added and committed.
- Step 4: the dialog "Git User Name Is Not Defined" with Name, E-mail,
  "Set properties globally" and "Set and Commit"; the commit is made with
  the values entered.
- Step 5: an error balloon with the hook's output; with the hooks off
  the commit succeeds.
- Step 6: a warning about committing in detached HEAD before the commit.

## Commands
`vcs.commit.run {"message": ...}`, `vcs.commit.options`, `vcs.state`,
`dialog.confirm`, `workbench.state` (`dialog`, notifications).

## Source
https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `VcsBundle`,
`GitBundle` (commit errors, user name dialog), `plugins/git4idea` commit
options. Dialog layouts are to verify against a reference screenshot.
