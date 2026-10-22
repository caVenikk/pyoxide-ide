# Copy, transposed view, text view, aggregates

## Steps
1. Select a block of cells; copy as CSV, TSV, Markdown, JSON, SQL INSERT, SQL UPDATE, WHERE condition and HTML.
2. Switch to the transposed view for one row.
3. Switch to the text view.
4. Select a numeric column and read the aggregates.

## Expected
- Each copy format produces correct quoting and escaping (commas, quotes, newlines, NULL).
- The transposed view shows one row as name and value pairs.
- The text view renders the page as a text table.
- Aggregates show count, sum, average, minimum, maximum and median of the selection.

## Source
observed
