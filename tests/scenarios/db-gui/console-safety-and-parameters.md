# Dangerous statements and parameters

## Steps
1. Execute `delete from t` and `update t set a = 1` (no WHERE).
2. Execute `select * from t where id = :id`, enter a value, run again.
3. Execute a statement with `?` placeholders (SQLite) and one with `$1` (PostgreSQL).

## Expected
- A confirmation offers Execute, Skip or Cancel before a DELETE or UPDATE without WHERE.
- Named and positional parameters open an input dialog; the values are remembered for the next run.
- Values are sent as bound parameters, not pasted into the SQL text.

## Source
observed; engine row bind-parameters
