# 0002. Database layer contract

- Status: accepted
- Date: 2026-10-09

## Context
The database client must support Oracle, PostgreSQL and SQLite in the MVP,
behind one interface used by the UI, the CLI and commands. The UI needs typed
values, paging through large results without loading them, cancellation of
long statements, per-call timeouts and schema metadata.

## Decision
`db-core` defines the contract and has no driver dependencies.

**Values.** `Value` is a closed enum: `Null, Bool, Int(i64), Float(f64),
Decimal(String), Text, Bytes, Date, Time, Timestamp, TimestampTz, Interval,
Uuid, Json, Array, Truncated { prefix, full_len }, Other { type_name, text }`.
Exact numbers that do not fit `i64` stay as text (`Decimal`), so Oracle
`NUMBER(38)` and PostgreSQL `numeric` are never rounded. Dates use `chrono`.
Values longer than `ExecOptions::max_value_bytes` (default 1 MiB) arrive as
`Truncated`: the prefix has at most that many bytes (text is cut on a
character boundary), and `full_len` is the full length when it is known, in
the backend's own unit: bytes in general, but for Oracle CLOB and NCLOB
values it can be the LOB length in characters (UTF-16 code units), which is
what the server reports without reading the whole value. `Column` carries
`type_name` (backend type) and `kind` (`ValueKind`) for alignment and
editors. Values serialize to JSON with a `type` tag; bytes as hex.

**Statements and paging.** `Connection::execute(sql, params, opts)` returns
`Outcome::Affected(n)` or `Outcome::Rows(Box<dyn Cursor>)`. A cursor is read
with `next_page(n)`; an empty page means the end. When the backend reports
an error after rows of a page were read (for example while reading ahead to
detect the end), a driver may return those rows first and the error on the
next call; a driver that cannot keep them returns the error at once. Either
way an error ends the cursor, and later calls report an error again. Binds
are positional in the dialect's syntax (`:1`, `$1`, `?1`).

**One open cursor per connection.** While a cursor is open, `execute` on that
connection fails with `ErrorKind::Busy`; `cancel`, `commit`, `rollback` and
`close` still work. Rationale: each backend serializes a session differently
(Oracle TTC, PostgreSQL pipelining with backpressure, a single SQLite worker
thread); allowing interleaving makes deadlocks possible when the same task
pages a cursor and runs another statement. The IDE uses a separate connection
for introspection, like other database tools do.

**Closing early stops server work.** Closing a cursor that is not exhausted
cancels or closes it on the server; it never silently drains the rest.
`Cursor::close` returns once the connection is free. Dropping a cursor may
free the connection asynchronously, while the driver stops the server work in
the background; an `execute` issued right after the drop then waits briefly
for that cleanup (`BusyFlag::acquire_after_cleanup`) instead of failing with
`Busy`.

**Cancel and timeouts.** `Connection::cancel()` can be called from another
task. `ExecOptions::timeout` applies to `execute` and to each `next_page`.
`db_core::timeout::with_timeout` implements the common policy: on timeout,
send a cancel, give the statement a grace period (5 s), then drop it. A
dropped future must leave the connection usable or mark it broken
(`ErrorKind::ConnectionLost`), never half-way through the protocol.

**Transactions.** `TxMode::Auto` (default; every statement commits) or
`Manual` (`commit`/`rollback`). `commit` and `rollback` may end an open
cursor (ending a transaction closes PostgreSQL portals), so a cursor is read
to the end or closed before the transaction it runs in is ended.

**Errors.** `Error { kind, code, message }`. `kind` is backend-neutral
(`Config, Network, Tls, Auth, Connect, Query, Timeout, Cancelled, Busy,
ConnectionLost, Unsupported, Unavailable, Internal`); `code` is the backend's
code (`ORA-12514`, SQLSTATE `28P01`). Messages never contain secrets;
`Error::scrub` removes a known secret defensively.

**Secrets.** `ConnectParams.password` and the wallet password are `Secret`:
redacted in `Debug`, not serializable, zeroed on drop.

**Introspection.** `schema::Introspector` (schemas, objects, columns,
indexes, constraints, source) with one implementation per dialect in
`db-core::schema`, written only against `Connection::query` with bound
parameters. Both Oracle drivers share the same SQL.

## Consequences
- Drivers stay small: they map types and errors and implement paging/cancel.
- The data grid can show the first page immediately and fetch on scroll.
- A second connection is needed for metadata while a grid is open. SQLite
  `:memory:` databases are per connection: share one through a
  `file:name?mode=memory&cache=shared` URI, or introspect on the same
  connection after closing its cursor.
- Backends that cannot cancel (see ADR 0003) rely on dropping the call and
  reconnecting; the contract makes that safe.
