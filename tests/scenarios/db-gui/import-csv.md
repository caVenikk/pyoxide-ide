# Import a CSV file into a table

## Steps
1. Prepare a CSV with a header, quoted fields containing commas and line breaks, an empty field and a column not in the table.
2. Table menu > Import; choose the file; check delimiter, header, NULL text, mapping; look at the preview.
3. Run it with a value that violates a constraint in the last row.

## Expected
- The mapping table pairs file columns with table columns; unmatched ones can be skipped.
- Empty fields become NULL when chosen.
- The import runs in one transaction: with the violation nothing is inserted and the report names the row and the reason.
- A successful import reports the row count.

## Source
https://www.jetbrains.com/help/pycharm/export-data.html; observed
