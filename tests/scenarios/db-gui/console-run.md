# Run statements in a console

## Steps
1. New > Console on a data source. Type three statements on separate lines, put the caret in the second, press Cmd/Ctrl+Enter.
2. Select all three and execute.
3. Put the caret in an empty line far from any statement and execute.
4. Write a function with `$$ ... $$` (PostgreSQL) or a PL/SQL block (Oracle) containing semicolons; execute it.
5. Execute a script whose second statement fails.

## Expected
- With the caret in a statement only that statement runs. Where the caret is in nested statements (a subquery) a small picker offers the choices; with the caret outside any statement nothing runs.
- A selection runs as separate statements, one after another.
- Statements are separated by a lexer: semicolons inside strings, comments, dollar-quoted bodies and PL/SQL blocks do not split.
- When one statement of a script fails the error shows the position in the editor and the user chooses to continue, skip or stop.
- Each statement leaves one line in the output with the schema echo, the row count or affected count and the time.

## Source
https://www.jetbrains.com/help/pycharm/run-a-query.html; observed
