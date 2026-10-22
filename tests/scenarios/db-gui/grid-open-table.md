# Open a table and page through it

## Steps
1. Double-click a table with 10 000 rows.
2. Read the page bar; go to next, previous, last, first page; change the page size.
3. Click the count link.
4. Click a column header to sort; type `id > 100` into WHERE and `name desc` into ORDER BY, press Enter.
5. Enter a wrong WHERE.

## Expected
- The first page (500 rows by default) appears without reading the table; the bar shows "1 - 500" with an open total until the count is asked for.
- Later pages continue where the previous ended; page size is a setting (10, 100, 500, 1000, custom, no limit).
- The count runs as a separate `count(*)` over the same query.
- Sorting by a header orders on the server; WHERE and ORDER BY text is inserted as written.
- A wrong condition shows the server's message and keeps the previous rows.
- Column headers show type; key columns are marked.

## Source
https://www.jetbrains.com/help/pycharm/rows.html; observed
