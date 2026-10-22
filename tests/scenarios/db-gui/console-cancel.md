# Stop a running statement

## Steps
1. Run `select pg_sleep(60)` (PostgreSQL), a long recursive query (SQLite) or a PL/SQL sleep (Oracle).
2. Press Stop.
3. Run `select 1` in the same console.
4. Repeat while the grid is fetching a large result.

## Expected
- The statement ends within a second on PostgreSQL and SQLite and the output says it was cancelled.
- The session stays usable; the next statement runs.
- With the thin Oracle driver (no server cancel) the session is reconnected and, if a manual transaction was open, the user is told it was lost.
- The UI stays responsive during the whole time.

## Source
observed; ADR 0002, 0003
