# Export a result to a file

## Steps
1. Run a query returning 100 000 rows (page size 500).
2. Export as CSV with a header and the NULL text `\N`; then as JSON, SQL INSERT and Markdown.
3. Cancel an export midway.

## Expected
- The export re-runs the query without the page limit and writes the whole result, streaming to the file (memory stays flat).
- The row count in the file is 100 000 plus the header.
- Options of the format (delimiter, quote, NULL text, header) are honoured.
- A cancelled export stops the query on the server and leaves no half-written file claiming success.

## Source
https://www.jetbrains.com/help/pycharm/export-data.html; observed
