# Rows of the project tree

## Steps
1. Open a folder with `src/pkg/mod.py`, `src/app.py` and `README.md` and
   expand `src` and `src/pkg`.
2. Open a folder whose name and location together are wider than the
   project tool window (or narrow the tool window until the root row is
   cut). Select the root row, then a short row.
3. Point at the root row and move the pointer onto its rest over the
   editor; click it, double-click it, right-click it. Point at a short
   row; move the pointer out of the tree.
4. Click a short row, press Up until the root row is selected, then click
   in the editor.
5. Swipe the tree sideways (or Shift + wheel) with the cut root row
   selected, to the end and back.

## Expected
- Step 1: a thin vertical line (the theme's indent guide colour, faint)
  runs below each open folder at the centre of its chevron, through the
  rows inside it down to its last visible row: one line below the root,
  one below `src`, one below `src/pkg`. Closed folders and files draw
  none of their own. The selected row's lines look like the others.
- Step 2: the root row's selection starts where every row's does and
  runs to the right edge of the tree, leaving about a pixel, with square
  right corners; the text is cut at the edge. A short row's selection
  keeps its inset and rounded corners on both sides. A root row that fits
  looks like any other row.
- Step 3: 10 ms later the rest of the root row shows over the tree's
  edge, over the editor, as one continuous row: no seam where it meets
  the row's selection or hover, the same background (the row stays
  hovered while it shows), the text continuing pixel for pixel at the
  same height, and a 1 px border outside the row above, below and at the
  rounded right end, none where it joins the row. It stays
  while the pointer is on it; a click there selects the root row, a
  double click collapses or expands it, a right click opens its context
  menu. It shows only for a row that is cut and fully in view, goes when
  the pointer is on a short row, and goes when the pointer leaves both
  the tree and the rest.
- Step 4: selecting the cut root row with the keyboard shows its rest
  while the tree has the focus; it goes when the focus moves to the
  editor or the selection moves on.
- Step 5: the rows scroll sideways until the widest row's text ends
  with a little room (the cell's 4 px and the tree's 12 px inset) and no
  further; the highlight of the selected or hovered row starts 12 px in
  (at the row's own chevron when that is further left) and, once the row's
  text fits the view, ends 12 px before the view's edge with rounded
  corners; no rest of a row shows over the edge when it fits.

## Commands
`project.open-folder {"path"}`, `project.tree.reveal {"path"}`,
`project.tree.up`, `project.tree.down`, `workbench.state`
(`tree.rows[].guides`, `tree.rows[].overflows`, `tree.expanded_row`,
`tree.scroll_left`).

## Source
IntelliJ Platform Community at `pycharm/2026.2.3` (Apache-2.0):
`ClassicPainter` (indent lines, `Tree.hash`), `UISettings`
(`showTreeIndentGuides`), `TreeExpandableItemsHandler`,
`AbstractExpandableItemsHandler`, `DefaultTreeUI` (highlight extent),
`RenderingHelper`; the mechanism is `ui::expandable`, shared with the
Database tree (`tests/scenarios/db-gui/tree-row-expansion.md`); observed in PyCharm 2026.2.3 with "Show
indent guides" on.
