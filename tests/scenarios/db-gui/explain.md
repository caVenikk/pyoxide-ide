# Explain plan

## Steps
1. In a console run Explain on a query with a join and a filter.
2. Run Explain with execution (analyze) on a SELECT.

## Expected
- A plan appears as a tree with node, cost, row estimate and, with analyze, actual time and rows; the raw text is available.
- Analyze runs the statement; on a read-only data source only SELECT is allowed to run it.

## Source
PostgreSQL EXPLAIN docs; Oracle DBMS_XPLAN docs; SQLite EXPLAIN QUERY PLAN docs
