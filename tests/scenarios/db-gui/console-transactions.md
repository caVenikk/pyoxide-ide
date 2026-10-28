# Manual transactions

## Steps
1. Switch a console to manual mode.
2. Run `insert ...; select 1/0; insert ...` (selected together).
3. Look at the pending indicator, then Commit; check the data from another client.
4. Run another insert, close the console.
5. Switch back to auto mode with a pending transaction.

## Expected
- The failed statement does not abort the transaction: the first insert is kept, the transaction continues.
- A visible indicator shows the number of pending changes; Commit and Rollback are available.
- Closing a console or disconnecting with a pending transaction asks: commit, roll back or cancel.
- Switching to auto mode with a pending transaction asks the same.

## Source
https://www.jetbrains.com/help/pycharm/managing-connection-sessions.html; observed

## Engine differences
- PostgreSQL: all four isolation levels; Database Default resets `default_transaction_isolation`. Savepoints keep a transaction usable after a failed statement.
- Oracle: Read Committed and Serializable (`ALTER SESSION SET ISOLATION_LEVEL`); Repeatable Read is refused with a message. DDL commits the open transaction, so the pending count resets.
- SQLite: no isolation menu items (they are disabled); transactions have no levels.
- Closing a console tab does not prompt (the connection session stays); disconnecting and switching to auto mode do.
