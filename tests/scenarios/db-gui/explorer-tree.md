# Browse objects in the Database tool window

## Steps
1. Connect to a database with several schemas, tables, views, a partitioned table, a function and a sequence.
2. Expand data source > database > schema.
3. Expand a table: columns, keys, foreign keys, indexes, triggers.
4. Stop the server and expand nodes again.
5. Type letters in the tree (speed search); filter to views only.

## Expected
- Levels: data source, database, schema, groups by kind (tables, views, routines, sequences, ...), objects, groups of children. Empty groups are hidden.
- Partitions are nested under their parent table.
- Column rows show the type; primary key, foreign key and NOT NULL are marked.
- Expansion is instant and works from the local model, also with the server stopped; only Refresh talks to the server.
- Speed search jumps to matches; the kind filter hides other kinds.

## Source
https://www.jetbrains.com/help/pycharm/database-tool-window.html; observed
