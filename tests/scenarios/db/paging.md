# Page through a large result

## Steps
1. Run `SELECT` over a table with 100,000 rows.
2. Read the first page, then two more pages.
3. Close the result before the end; run another statement on the same
   connection.

## Expected
- The first page (500 rows by default) appears without reading the whole
  result; later pages continue where the previous one ended.
- Closing early stops the server-side work; the next statement runs at once.
- A statement that returns no rows reports the number of affected rows.

## Source
https://www.jetbrains.com/help/pycharm/rows.html (page size, default 500)
