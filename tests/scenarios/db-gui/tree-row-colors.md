# Colours of the Database tree's rows

## Steps
1. Open the Database tool window with a data source that has a model and
   expand a schema and its tables.
2. Right-click a table, open Set Color and choose Yellow. Do the same for
   another table with Green and for the data source with Blue.
3. Click a coloured table, then another row; point at a coloured row.
4. Right-click the schema, Set Color, Teal, then Set Color, Apply to
   Children. Right-click one table, Set Color, No Color; then Set Color,
   Auto.
5. Restart the IDE.

## Expected
- Step 2: the menu is Auto (Current), a separator, No Color, Blue, Teal,
  Green, Yellow, Orange, Red, Purple, Gray (each with its circle), Custom
  (disabled), a separator, Share through VCS (disabled), Apply to Children,
  a separator, Settings. The chosen row is painted with its colour from
  the tree's left edge to its right edge, with square corners, and the
  text keeps its colour; other rows do not change.
- Step 3: a selected row keeps its colour, the selection is drawn over it
  semi-transparent; the pointer's row keeps it likewise under the hover
  highlight.
- Step 4: after Apply to Children every row below the schema (tables,
  groups, columns) has the schema's colour; No Color leaves that table
  without one; Auto gives it the schema's again. Apply to Children is
  disabled for a row with no colour.
- Step 5: the colours are as before (they are kept in the data source's
  settings).

## Commands
`db.explorer.set-color {"color"}`, `db.explorer.apply-color-to-children`,
`db.state` (`explorer.tree.rows[].color`).

## Source
Observed in PyCharm 2026.2.3 (Set Color menu of the Database tool window,
coloured rows).
