# Amend the last commit

## Steps
1. Make a commit "First"; type a draft message `draft` in the Commit tool
   window.
2. Tick Amend (Ctrl+Alt+M on macOS, Alt+M on Windows and Linux).
3. Change the message to `First, amended`, tick one changed file and
   press the commit button.
4. Run `git log -2` in a terminal.
5. Tick Amend again and untick it.

## Expected
- Step 2: the last commit's message is loaded into the field; the
  buttons read "Amend Commit" and "Amend Commit and Push…".
- Step 3: the commit is replaced; a balloon reports it.
- Step 4: HEAD has the new message and the same parent as the old commit.
- Step 5: unticking puts the draft `draft` back.

## Commands
`vcs.commit.toggle-amend`, `vcs.commit.run {"message": ...}`,
`vcs.state` (amend, message).

## Source
https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`Vcs.ToggleAmendCommitMode`, `VcsBundle` (amend strings).
