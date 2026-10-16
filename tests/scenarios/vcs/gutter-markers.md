# Gutter change markers

## Steps
1. In a tracked file change line 3, insert two lines after line 5 and
   delete line 9.
2. Hover each marker; click the marker of line 3.
3. In the popup use Next Change and Previous Change, then Copy.
4. Click Rollback Lines; press Cmd+Z (macOS) or Ctrl+Z.
5. Put the caret at the top and press Ctrl+Alt+Shift+Down twice, then
   Ctrl+Alt+Shift+Up.
6. Change only the indentation of a line.

## Expected
- Step 1: a 4 px pill next to the line numbers: modified (blue) on line
  3, added (green) on the inserted lines, a short deleted pill (grey) at
  the boundary of the removed line; a mark per change in the error
  stripe.
- Step 2: the hovered marker widens; the popup shows Previous Change,
  Next Change, Rollback Lines, Show Diff for Lines, Copy and the old
  text with word highlights.
- Step 3: the popup moves between changes; Copy puts the old text on the
  clipboard.
- Step 4: the old text is back and the marker gone; undo brings the
  change back.
- Step 5: the caret moves to each change in order.
- Step 6: the marker uses the whitespace-only colour.

## Commands
`vcs.next-change`, `vcs.previous-change`, `vcs.show-current-change`,
`vcs.rollback-lines`, `vcs.copy-change-lines`, `vcs.show-diff-lines`,
`marker-popup.close`, `git.status`, `workbench.state` (`vcs.open_files[]
.markers`: the ranges and their pieces; `vcs.marker_popup`),
`editor.state` (decorations of the group `vcs`).

## Source
https://www.jetbrains.com/help/pycharm/editor-gutter.html,
https://www.jetbrains.com/help/pycharm/undo-changes.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `LineStatusMarkerDrawUtil`,
`LineStatusMarkerPopupPanel`. The popup look is to verify against a
reference screenshot.
