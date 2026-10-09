# Schema metadata

## Steps
1. Create a schema with a table (primary key, unique key, foreign key, check
   constraint, an index, a column comment), a view, a sequence and a routine.
2. List schemas, then objects of the schema, then columns, indexes,
   constraints and the view/routine source.

## Expected
- Every object appears under the right kind; system schemas are flagged.
- Columns have full types (`VARCHAR2(40 CHAR)`, `numeric(10,2)`), nullability,
  defaults and comments; constraints list their columns and references.
- Names with mixed case or spaces work (values are bound, never spliced).

## Source
Oracle Database Reference (ALL_* views), PostgreSQL catalogs, SQLite
`sqlite_schema` and PRAGMA documentation.
