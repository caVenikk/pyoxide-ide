# Diff viewer

## Steps
1. Modify two tracked files in several places; in the Commit tool window
   double-click the first.
2. Press F7 twice and Shift+F7.
3. Type on the right side.
4. Press Ctrl+Shift+Right (macOS) or Alt+Shift+Right.
5. Select a file and press Cmd+D / Ctrl+D; press Enter on another.
6. Open the diff of a file whose change is only line endings on Windows.

## Expected
- Step 1: a preview editor tab "Commit: <file>" (grey title); side by
  side: the committed version on the left (a lock, the short hash and
  the path in grey, read-only, its line numbers next to the divider),
  "Current version" with the file's check box on the right (editable);
  changed lines highlighted with word highlights, the divider joining
  them; a count "{n} differences" and "2/3 files" with arrows in the
  toolbar. (Unchanged fragments collapsed with 4 lines of context come
  with folding, #32.)
- Step 2: the caret moves to the next, next and previous difference;
  at the last one F7 shows "Press again to go to the next file", and a
  second F7 opens the next file's diff.
- Step 3: the right side edits the file; the gutter markers and the
  Commit tree follow.
- Step 4: the next file's diff opens.
- Step 5: both open the diff.
- Step 6: no false differences (the base text goes through Git's
  filters).

## Commands
`vcs.show-diff {"path": ...}`, `diff.next`, `diff.previous`,
`diff.next-file`, `diff.previous-file`, `diff.focus-opposite`,
`diff.jump-to-source`, `diff.set-ignore-policy`, `workbench.state`
(`tabs`, `diff`).

## Source
https://www.jetbrains.com/help/pycharm/comparing-file-versions.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`platform/diff-impl` (side-by-side viewer, `DiffBundle`). Line numbers and
the toolbar are to verify against a reference screenshot.
