# Cancel a running statement and statement timeouts

## Steps
1. Run a statement that takes a minute (`pg_sleep(60)`,
   `DBMS_SESSION.SLEEP(60)`, a recursive CTE in SQLite).
2. Cancel it from another task.
3. Run the same statement with a 2 second timeout.

## Expected
- Cancel ends the statement within a few seconds with a "cancelled" error;
  the connection stays usable (or is reported as lost and reopened for
  backends that cannot cancel).
- The timeout ends the statement with a "timed out" error.

## Source
https://www.jetbrains.com/help/pycharm/run-a-query.html (Cancel Running Statements)
