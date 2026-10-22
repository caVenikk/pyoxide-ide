# Result tabs and the output log

## Steps
1. Run `select 1`, then `select 2`: the first tab is reused.
2. Pin the tab, run `select 3`: a new tab opens.
3. Rename a tab, run again: the renamed tab is kept.
4. Run a statement that returns two result sets (a Oracle procedure with ref cursors, or a script of two selects selected together).
5. Run a procedure that raises a notice.

## Expected
- An unpinned, unrenamed tab is reused; pinned and renamed tabs stay.
- Every result set has its own tab, numbered when one statement yields several.
- Notices and warnings from the server appear in the output log.
- A statement without rows (insert) shows the affected row count in the output, not an empty grid.

## Source
https://www.jetbrains.com/help/pycharm/viewing-query-results.html; observed
