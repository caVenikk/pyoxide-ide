# DDL and generated SQL from the tree

## Steps
1. Select a table: show its DDL; Copy DDL.
2. Select a view and a function: show their source.
3. Use SQL Scripts to generate SELECT, INSERT, UPDATE and DELETE for a table with a mixed-case column name.

## Expected
- DDL includes columns, keys, indexes and comments as the engine's own functions or catalog give them.
- Source of views and routines is loaded on demand.
- Generated statements quote identifiers when needed and use every column.

## Source
observed; engine row object-source-and-ddl
