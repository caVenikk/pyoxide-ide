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
