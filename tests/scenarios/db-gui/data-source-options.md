# Session options of a data source

## Steps
1. Open Options of a data source: set read-only, manual transaction mode, a startup script (`set application_name = 'x'` or an equivalent), a time zone, keep-alive and auto-disconnect.
2. Connect and run `show application_name` / `select current_setting('TimeZone')` in a console.
3. With read-only on, run an `insert`.
4. Leave a console idle past the auto-disconnect time with no open transaction, then run a query.

## Expected
- The startup script runs on every new connection (consoles, grid reads, refresh) before the first user statement.
- The time zone applies to the session.
- The write fails with the server's own error (the session is read-only), not with a client-side block.
- After auto-disconnect the next run reconnects without an error dialog; with an open transaction the user is asked first.

## Source
observed; engine rows startup-script-keep-alive-auto-disconnect, read-only-connections
