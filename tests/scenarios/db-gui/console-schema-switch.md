# Current schema of a console

## Steps
1. Open a console on a data source with two schemas; read the schema shown in the toolbar.
2. Pick the other schema in the list and run `select * from some_table` (a name only in that schema).
3. Type `set search_path = public` (PostgreSQL) or `alter session set current_schema = X` (Oracle) and run it.

## Expected
- Choosing a schema changes the session's search path; unqualified names resolve there.
- A manual `set` is detected and the toolbar shows the new schema.
- Completion and name resolution in the editor follow the current schema.

## Source
observed

## Engine differences
- PostgreSQL: the session `search_path` is set to the chosen schema.
- Oracle: `ALTER SESSION SET CURRENT_SCHEMA`; the chip is re-read after a manual `ALTER SESSION`.
- SQLite: there are no schemas to switch; the schema list is empty.
