# Commit

## Steps
1. Modify two tracked files; stage a third change with `git add` in a
   terminal; create an unversioned file.
2. Press Cmd+K (macOS) or Ctrl+K.
3. Type `Fix` and press Cmd+Enter / Ctrl+Enter.
4. Run `git log -1 --stat` and `git status` in a terminal.
5. Click the Commit stripe button with no changes left.

## Expected
- Step 2: the Commit tool window opens in the left area; the tree has
  "Changes" (bold) with a grey count and the changed files checked,
  "Unversioned Files" unchecked; the legend reads like "3 modified"; the
  focus is in the message field ("Commit Message" placeholder).
- Step 3: a balloon "3 files committed" with "Fix"; the tree and the
  message are cleared.
- Step 4: the commit has the message and only the checked files; a
  staged change of a file that was unchecked stays staged.
- Step 5: the empty state of the Commit tool window.

## Commands
`vcs.commit`, `vcs.commit.run {"message": ...}`, `vcs.state` (commit
tree, inclusion, message), `git.status`.

## Source
https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`platform/vcs-impl` (Commit tool window, `commit.legend.*` strings),
`plugins/git4idea` (commit). The window and empty state are to verify
against a reference screenshot.
