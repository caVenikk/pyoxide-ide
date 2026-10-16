# Roll back files

## Steps
1. Modify a tracked file that is open in the editor; `git add` a new
   file.
2. In the Commit tool window select both and press Cmd+Alt+Z (macOS) or
   Ctrl+Alt+Z.
3. Leave "Delete local copies of added files" unticked; press Rollback.
4. Add the new file again; roll it back with the check box ticked.

## Expected
- Step 2: the dialog "Rollback Changes" lists both files with check
  boxes and the check box "Delete local copies of added files".
- Step 3: the modified file has the committed content again and its open
  editor reloads; the added file becomes unversioned and stays on disk;
  the tree updates.
- Step 4: the added file is removed from the index and deleted.

## Commands
`vcs.rollback {"paths": [...], "delete_added": ...}`, `vcs.state`,
`git.status`.

## Source
https://www.jetbrains.com/help/pycharm/undo-changes.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `ChangesView.Revert`,
`VcsBundle` (`changes.checkbox.delete.locally.added.files`). The dialog is
to verify against a reference screenshot.
