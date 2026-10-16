# File status colours

## Steps
1. Open a Git project. Type in the tracked `a.py` without saving; undo
   the edit; edit and save it.
2. Create `b.py` in the project tree.
3. Run `git add b.py` in a terminal.
4. Add `logs/` and `.env` to `.gitignore`; create both.
5. Rename a tracked file with `git mv`.
6. Commit everything outside the IDE.
7. Mark a folder as excluded that is also ignored.

## Expected
- Step 1: `a.py` turns the modified colour (blue) in the project tree and
  in its editor tab as soon as it is edited, back to normal when the text
  equals the committed one again, and modified after the save.
- Step 2: `b.py` is in the unversioned colour (red); its folder is not
  coloured.
- Step 3: `b.py` turns the added colour (green) within about half a
  second.
- Step 4: `logs/`, everything below it and `.env` are in the ignored
  colour.
- Step 5: the renamed file is in the modified colour.
- Step 6: every name is back to the normal colour.
- Step 7: the excluded folder keeps the excluded style.

## Commands
`git.status`, `workbench.state` (tree rows and tabs with their status).

## Source
https://www.jetbrains.com/help/pycharm/file-status-highlights.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`FileStatusManagerImpl`, `VcsConfiguration` (`SHOW_DIRTY_RECURSIVELY`),
`FILESTATUS_*` keys in `islands/IslandSchemeDark.xml`. Whether ignored
icons are dimmed too is to verify against a reference screenshot.
