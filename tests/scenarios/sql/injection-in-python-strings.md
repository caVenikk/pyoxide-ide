# SQL in Python strings

## Steps
1. In a Python file write `# language=SQL` above a string `"select id from users where name = %s"`.
2. Write `cur.execute("select * from orders where id = " + str(i))` and an f-string `f"select * from {table} where x = 1"` passed to `execute`.
3. Write `query = "select name from users"` and `msg = "select a book from the shelf"`.
4. Put the caret in an injected string and complete a column; make a typo in a table name.
5. Invoke "Don't inject" on the first string; invoke "Inject SQL" on `msg`.
6. Open a file with a string containing `\n` escapes and non-ASCII text; complete after a `.` on the second line.

## Expected
- Steps 1-3: the first, the `execute` argument and `query` strings are SQL (colours, completion); `msg` is not; the f-string and the concatenation are SQL with holes.
- Step 4: completion lists columns; the unresolved name is marked only in a string without holes and with a scope.
- Step 5: the injection switches off and on; the choice survives a restart.
- Step 6: the completion inserts at the right place in the Python source; escapes in the string stay intact.
- `%s` and `?` are parameters, not holes.

## Source
Public documentation of language injections; Apache-2.0 sources of the reference's Python injection (see `replica/recon.md`, area db-gui, "SQL language").
