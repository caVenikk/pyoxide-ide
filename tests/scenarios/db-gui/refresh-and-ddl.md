# Refresh and changes made outside the IDE

## Steps
1. In another client add a column, add a table, drop a table.
2. Refresh the data source in the tree; then refresh only one schema.
3. In a console run `create table t1 (...)` and expand the schema without refreshing.
4. Cancel a refresh midway.

## Expected
- Refresh shows the new column and table and removes the dropped table.
- A repeated refresh of an unchanged database is quicker than the first full load and changes nothing visible.
- DDL run in the console updates the tree for the named objects without a full refresh.
- A refresh does not commit or roll back a transaction open in a console (it uses its own connection).
- A cancelled refresh leaves the tree as it was, not a mix of old and new.

## Source
PostgreSQL system catalogs docs; observed
