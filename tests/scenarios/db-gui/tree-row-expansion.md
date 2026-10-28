# Wide rows of the Database tree

## Steps
1. Open the Database tool window on a data source with a cached model and
   expand `tables`, `customers` and its `indexes`, so that a row such as
   an index with a long definition is wider than the tool window.
2. Point at that row; then at a short row; then move the pointer out of
   the tree.
3. Swipe the tree sideways (or Shift + wheel) to the end and back; point at
   the long row at the end.
4. Select the long row with the keyboard while the tree has the focus.

## Expected
- Step 2: 10 ms after the pointer comes to the long row, the rest of the row
  shows over the tool window's edge (over the editor beside it) as one
  continuous row: the same background, the text continuing at the same
  height, a 1 px border above, below and at the rounded right end, none
  where it joins the row. It stays while the pointer is on it and goes
  when the pointer is on a short row or leaves both the tree and the
  rest. Rows that fit show no rest. The pointer's row has the hover
  background, the same shape as the selection (12 px inset on both sides,
  rounded corners); a row that is cut reaches the edge but for a pixel
  with a square end.
- Step 3: the rows scroll sideways until the widest row ends with a little
  room; scrolled so far that the long row fits, its highlight ends inside
  the view with rounded corners and no rest shows.
- Step 4: the rest shows while the tree has the focus and the row stays
  selected.

## Commands
`db.explorer.expand {"path"}`, `db.state` (`explorer.tree.rows`).

## Source
The same as `tests/scenarios/editor/project-tree-rows.md`
(`TreeExpandableItemsHandler`, `AbstractExpandableItemsHandler`,
`DefaultTreeUI`, `RenderingHelper`): `ui::expandable`.
