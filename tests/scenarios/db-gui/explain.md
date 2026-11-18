# Explain plan

## Steps
1. In a console run Explain on a query with a join and a filter.
2. Select an operation, then move with Up and Down. Collapse with Left and expand with Right.
3. Switch to Raw using Tab. On PostgreSQL, compare the QUERY PLAN cells and their indentation with the server's text output.
4. Switch back, then press Escape and type in the SQL editor. Close the plan tab.
5. Run Explain with execution (analyze) on a SELECT and switch views again.

## Expected
- A plan appears as a tree with node, cost, row estimate and, with analyze, actual time and rows; the raw text is available.
- Raw and Operations Tree use the same result, including any rows beyond the first cursor page. Switching views keeps the selected tree operation and sends no SQL.
- Analyze runs the statement; on a read-only data source only SELECT is allowed to run it.

## Source
PostgreSQL EXPLAIN docs; Oracle DBMS_XPLAN docs; SQLite EXPLAIN QUERY PLAN docs
