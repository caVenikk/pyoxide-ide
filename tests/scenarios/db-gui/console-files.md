# Consoles are files; SQL files bound to a data source

## Steps
1. Write text in a console, close the IDE, open it again.
2. Open a project `.sql` file, attach it to a data source and schema, and execute a statement.
3. Open the query history of the data source and search for a statement run yesterday.

## Expected
- The console text and its history survive a restart; each data source has its own consoles and history.
- A bound file behaves like a console; the binding is remembered per user, not committed.
- History lists text, time and duration and can be searched and re-inserted.

## Source
https://www.jetbrains.com/help/pycharm/query-consoles.html; observed
