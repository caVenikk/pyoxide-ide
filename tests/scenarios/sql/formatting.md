# SQL formatting

## Steps
1. Write a file with a long `select` with joins, a CTE, a `case`, a comment inside a clause, a string with `;`, and a PostgreSQL function with a `$$` body.
2. Reformat the file with the default code style; then with keywords in lower case and leading commas.
3. Reformat a statement that has a syntax error.
4. Reformat twice.

## Expected
- Clauses start on their own lines, indentation follows the style, keywords change case only.
- Comments, strings and the `$$` body are unchanged; the token sequence is the same apart from whitespace and keyword case.
- The statement with the error is left as it was.
- The second reformat changes nothing.

## Source
Public documentation of the SQL code style (verify the option list against the reference).
