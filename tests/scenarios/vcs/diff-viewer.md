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
- Step 1: an editor tab "Commit: <file>"; side by side: the committed
  version on the left (titled with the short hash, read-only), "Your
  version" on the right (editable); changed lines highlighted with word
  highlights; unchanged fragments collapsed with 4 lines of context; a
  count "{n} differences".
- Step 2: the caret moves to the next, next and previous difference.
- Step 3: the right side edits the file; the gutter markers and the
  Commit tree follow.
- Step 4: the next file's diff opens.
- Step 5: both open the diff.
- Step 6: no false differences (the base text goes through Git's
  filters).

## Commands
`vcs.show-diff {"path": ...}`, `diff.next`, `diff.previous`,
`diff.next-file`, `diff.previous-file`, `workbench.state` (active tab).

## Source
https://www.jetbrains.com/help/pycharm/comparing-file-versions.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`platform/diff-impl` (side-by-side viewer, `DiffBundle`). Line numbers and
the toolbar are to verify against a reference screenshot.
