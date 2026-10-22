# 0021. Database GUI architecture

- Status: proposed
- Date: 2026-10-22

## Context
ADR 0002 to 0004 gave the engine: `db-core` traits (`Driver`, `Connection`,
`Cursor`, typed `Value`s, cancel and timeouts, one open cursor per
connection, `Introspector`), three drivers, profiles with secrets and the
`db-service` that opens connections. Milestone M6 adds the interface: a
Database tool window with an object tree, SQL consoles, a data grid, export
and import, and later editing (recon: `replica/recon.md`, area `db-gui`).

The interface needs things the engine does not have yet:

- Work that never blocks the UI thread, can be cancelled, and survives a
  driver that hangs or cannot cancel (the thin Oracle driver reconnects).
- Several consoles per data source, each with its own transaction, while
  tree refresh and grid reads must not touch a user's open transaction.
- Results of any size shown one page at a time, with one open cursor per
  connection (ADR 0002).
- A metadata model that the tree and completion read instantly, that stays
  consistent when a refresh is cancelled and that is available right after
  start-up, before any connection.
- Splitting a script into statements by a lexer (dollar quotes, PL/SQL blocks,
  comments), not by searching for `;`.
- Layering (AGENTS.md, ADR 0005): UI crates depend on traits, not on drivers;
  platform crates know no language; `scripts/check-layers.py` checks it.

Observed behaviour of the reference that shapes the decision: every console
holds its own session; metadata refresh uses its own short connection; a
cancel reaches the server; pages hold 500 rows by default; the tree and
completion work from a local model; a failing statement in a manual
transaction does not abort the transaction.

## Options considered
**Where the logic lives.**
1. Logic and GPUI views in one `db-gui` crate. Simple, but the sessions, the
   model and the pager cannot be tested without a window, and the UI crate
   would drift towards driver types. The debugger and git work avoided this
   by splitting model and UI (ADR 0015, 0016).
2. Put sessions and model into `db-service`. It depends on every driver, so
   a UI crate would depend on drivers, against AGENTS.md.
3. New toolkit-free crates for text, model and sessions; a UI crate on top;
   `db-service` stays the only place that knows drivers. Chosen.

**Paging.**
- A. Cursor only: fast and exact for one forward reader, but only one cursor
  may be open per connection (ADR 0002), so several result tabs cannot all
  page, and going back needs a cache.
- B. Re-run with `LIMIT/OFFSET` (Oracle: `OFFSET .. FETCH`): any tab pages
  at any time, but needs a stable order, repeats the query's cost and works
  only for a single `SELECT`.
- C. Hybrid. Chosen: table grids use B on data connections; console results
  use A while their cursor is open, with a page cache, and B as a fallback for
  a single `SELECT`.

**Metadata.**
- Query the catalog on every expansion: simple, slow on large schemas, no
  offline tree, and the tree waits for the network. Rejected.
- A local model loaded by background passes and stored on disk. Chosen.

**Driver isolation in a separate process** (the reference starts a helper
process per driver): a crash then cannot take the IDE down. Not needed here:
the drivers are Rust or small C libraries behind a worker thread, every call
has a timeout and a cancel, and a hung connection is abandoned and replaced.
Revisit if a driver proves to crash.

## Decision
### Crates
All under `crates/platform/`; none depends on a language product.

| crate | toolkit | role |
|---|---|---|
| `db-core` (exists) | no | traits, values, errors, per-dialect catalog SQL. Gains bulk introspection and change tokens (below). No driver dependencies. |
| `sql-text` (new) | no | SQL lexing per dialect, statement splitting, statement at a position, statements in a range, parameter scan, statement classification (kind, `DELETE`/`UPDATE` without `WHERE`, DDL target), explain wrapper, formatting. No database dependency; its `Dialect` is its own enum. |
| `db-model` (new) | no | the metadata model: node arena, kinds, stable ids, merge of snapshots and deltas, name search, the on-disk cache format, completion and reference resolution over the model. Depends on `db-core` (types) and `sql-text`. No tokio. |
| `db-session` (new) | no | the asynchronous layer: runtime, data source handles, sessions, query jobs, the pager, refresh jobs, export and import jobs, console files and history. Depends on `db-core`, `db-model`, `sql-text`, `tokio`. |
| `db-service` (exists) | no | profiles, secrets and driver dispatch; implements `db_core::ConnectionSource` for `db-session`. Remains the only crate (besides the CLI) that depends on drivers. |
| `db-ui` (new) | GPUI | explorer tool window, data source dialog, console tab, results panel, grid element, value panel, export and import dialogs, settings page. Depends on `db-session`, `db-model`, `sql-text`, `db-core` (types only), `editor`, `ui`, `commands`. Never on a driver crate, `db-service` or `tokio`. |

The workbench composes: it builds the `ConnectionSource` from `db-service`,
starts the `DbRuntime`, registers the Database tool window, the commands, the
settings page and the keymap files.

`scripts/check-layers.py` gets these rules: `sql-text`, `db-model`,
`db-session` and `db-core` know no `gpui`; none of `sql-text`, `db-model`,
`db-session`, `db-ui` depends on `db-postgres`, `db-sqlite`,
`db-oracle-thin`, `db-oracle-odpi` or `db-service`; `db-ui` does not
depend on `tokio`; `editor` does not depend on `sql-text`.

### Contracts added to `db-core`
- `trait ConnectionSource { async fn open(&self, data_source: &DataSourceId, purpose: Purpose) -> Result<Arc<dyn Connection>>; }`
  with `Purpose = Console | Data | Meta`. Implemented in `db-service` (profile,
  secrets, driver), faked in tests. Connections are `Arc<dyn Connection>`:
  `Connection` methods take `&self`, so `cancel` can be called from another
  task while a statement runs (ADR 0002).
- `Introspector` gets bulk and incremental reads next to the per-object ones:
  `load(scope, level) -> Snapshot` (all schemas of the scope with their
  objects, columns, keys, indexes in few queries, not one per table),
  `source(..)` stays lazy, and
  `changes(since: &ChangeToken, scope) -> Result<Delta>` whose default is
  `Unsupported` (caller then reloads). `Snapshot` and `Delta` carry a new
  `ChangeToken`. Token per dialect, all from public documentation:
  PostgreSQL the catalog row versions (`xmin` of `pg_class`, `pg_attribute`,
  ... compared with a saved transaction id, the server start time, the list of
  live oids for deletions, the oldest active transaction as the safe bound);
  Oracle `LAST_DDL_TIME` of `ALL_OBJECTS`; SQLite `PRAGMA schema_version`
  (any change reloads the single schema). The first version ships full
  reload only; PostgreSQL deltas are a later issue.
- The model of ADR 0002 (`SchemaInfo`, `ObjectInfo`, ...) is extended where the
  tree needs it: function arguments, triggers and partitions with their
  parent, foreign key actions, identity columns, comments, owners.
  Additions are optional fields, so existing users do not change.

### Sessions and connections
- One `DbRuntime` (a small multi-thread tokio runtime with named threads) is
  owned by `db-session`. The UI never awaits: it calls `session.run(job)`
  and receives events through an `EventSink` callback (a `Fn(Event) + Send`
  that the UI crate turns into a GPUI foreground task and a `notify`).
- A `DataSourceHandle` per data source is a small state machine
  (`Disconnected -> Connecting -> Connected | Failed`, auto-disconnect on
  idle, reconnect on `ConnectionLost`). It owns:
  - **console sessions**: one `Session` per console (a connection, the
    transaction state from `db_core::transaction`, the current schema or search
    path, the running job). A setting switches to one shared session. A
    session is an actor: commands (`execute`, `commit`, `rollback`,
    `set_schema`, `close`) are handled in order; `cancel` is out of band.
  - **data connections**: up to two, opened lazily, used by table grids,
    `COUNT(*)` and export, so reading a table never waits for a console and
    never joins its transaction.
  - **meta connection**: opened for a refresh and closed after it, so a
    refresh neither sees nor ends a user's open transaction.
  Each connection sends `application_name` (or the Oracle and SQLite
  equivalents) naming the IDE, the data source and the purpose.
- Read-only data sources are enforced by the server where the engine allows it
  (session read-only mode), not only by disabling buttons.
- Startup script, time zone and keep-alive run on every new connection of
  any purpose.

### Query jobs and cancellation
- A job is a list of statements from `sql-text`. For each statement the
  session emits `Started`, then `Affected(n)` or `ResultOpened(tab, columns,
  first page)`, `Notice(text)` for server messages, `Finished(elapsed)` or
  `Failed(error, position)`. Timing is split into execute and fetch.
- Stop calls `Connection::cancel()`, waits the grace period of
  `db_core::timeout::with_timeout`, then abandons the call. A driver that
  cannot cancel (thin Oracle) marks the connection broken; the session
  reconnects, and if a manual transaction was open the UI is told it is gone
  (event `TransactionLost`). The target is a cancel observed in under one
  second on PostgreSQL and SQLite.
- Manual transaction mode wraps every user statement in a savepoint
  (`SAVEPOINT` / `RELEASE` / `ROLLBACK TO`, all three engines support it) so a
  failed statement leaves the transaction usable. The pending count is the
  number of data-changing statements since the last commit (classified by
  `sql-text`). Closing a console or disconnecting with a pending transaction
  asks first.
- Export and import are jobs too: progress events, cancel, and they run on a
  data connection (export) or a transaction of their own (import).

### Paged results
- `ResultPager` (in `db-session`) is the only way the grid obtains rows:
  `page(index) -> Page { rows, first_row, has_more }`, `count()` (a separate
  `COUNT(*)` over the same query on a data connection, only when asked),
  `close()`. The page size is a setting (default 500); the fetch size is
  separate.
- Table grids: the pager builds `SELECT` with the user's WHERE and ORDER BY
  text (inserted as written, validated by the server), uses the primary key as a
  tie-breaker order when no ORDER BY exists, and pages with offset on a data
  connection. Sort by header changes the ORDER BY on the server.
- Console results: the first page is read from the statement's cursor, which
  stays open while the session is idle; the next page continues it. A page
  cache (a few pages, a row cap) serves going back. Running another statement
  closes the cursor (ADR 0002); the older tab then pages through the offset
  fallback if its statement is a single `SELECT` or `WITH`, otherwise it shows
  a notice and offers a re-run.
- Large values arrive as `Value::Truncated` (ADR 0002); the grid shows the
  prefix and loads the full value on request by a keyed query. The row count
  of a page is bounded, memory of cached pages is bounded.

### Metadata model
- `db-model` holds, per data source, an immutable snapshot (`Arc<Model>`):
  nodes in an arena with structural sharing per schema. The UI reads the
  current `Arc` without locks. A refresh builds the new schema subtrees
  off-thread and swaps the `Arc` at the end, so a cancelled or failed refresh
  leaves the old model whole.
- Node ids are derived from kind and qualified name, not from server oids, so
  tree state (expanded, selected) survives a refresh; oids and change tokens
  are fields of the node.
- Per schema a **level** says how much is loaded: names only, structure
  (columns, keys, indexes, triggers), or everything including source text of
  views and routines (always lazy and cached separately). The level is chosen
  from the object count (the threshold is a setting; its default comes from
  measurement) and changed per schema from the tree.
- The **introspection scope** of a data source (current database or schema,
  all, a list, patterns; system schemas light and hidden by default) decides
  what a pass reads.
- **Disk cache**: one file per data source in the user cache directory
  (`<cache dir>/pyoxide/db/<data source id>/model.json`, source texts beside it),
  with a format version, written to a temporary file and renamed, never in the
  project folder, never containing secrets. A cache with another version or a
  failing parse is ignored and rebuilt. The tree and completion read it before
  any connection exists. The format may move to a binary encoding if
  measurements of a 10 000-table schema require it.
- DDL run in a console schedules a refresh of the objects the statement names
  (from `sql-text` classification), not a full pass.
- Refresh passes report progress (database, schema) and are cancellable.

### Data sources and files
- A data source in the first version is an ADR 0004 profile with a table of
  GUI options under it (`[profiles.<name>.gui]`: introspection scope, keep-alive,
  auto-disconnect, startup script, time zone, default transaction mode,
  schema levels). Secrets stay in the secret store. The CLI and the GUI see the
  same list.
- Sharing data sources through the project (a committable file with the
  connection facts and a personal part with user and options, keyed by an id)
  is a later step; this ADR keeps `DataSourceId` a stable string so the
  split can be added without changing sessions or the model.
- Consoles are `.sql` files under the configuration folder, one folder per
  data source, with a history file beside them. Binding any project `.sql` file to
  a data source and schema is stored in the personal project state (ADR
  0009). Both open in the normal `editor`; Execute is a command available
  where the active editor has a binding.

### UI
- Everything the user does is a registry command (ADR 0001): `db.explorer.*`,
  `db.console.*`, `db.grid.*`, `db.export.*`, `db.import.*` registered by
  `db-ui`; the engine's headless `db.*` commands stay in `db-service`. Keymaps
  go to `workbench/keymaps/*/db.toml` with their checks.
- The Database tool window is registered in the workbench tool window
  registry (right stripe). Console and table tabs use `workbench::tab_view`
  where they are editor tabs; results use a bottom panel.
- The grid is a virtualized element in `db-ui`: only visible rows and
  columns are laid out, text is shaped lazily and cached per cell, a page of
  500 rows and 50 columns scrolls at the display rate.
- SQL highlighting uses `syntax` with a SQL grammar (licence check in its
  issue) or the lexer of `sql-text` as a fallback; completion and references
  come from `db-model`.

### Performance budgets (to be added to `docs/performance.md` with the first slice)
Expanding a tree node: no database access, under one frame on a 10 000-table
model. Loading the cached model of 10 000 tables: measured in the cache
issue, target under 150 ms off the UI thread. The UI thread never waits on
a database call. A cancelled query is cancelled on the server in under
one second (PostgreSQL, SQLite). Scrolling a 500 x 50 page holds the display
rate.

### Testing
`sql-text` and `db-model` have unit and property tests (random token soup
never panics, ranges stay on character boundaries, merge is idempotent).
`db-session` runs its tests on SQLite in every run and on PostgreSQL and
Oracle as ignored tests using the containers of `docker-compose.yml`; the
`db-testkit` conformance suite gains cases for savepoints, cursor close on
the next statement, offset fallback and cancel while paging. A fake
`ConnectionSource` with scripted connections covers hang, connection loss and
slow fetch. UI crates are tested through their commands and snapshots.

## Consequences
- Five crates to add in an order that keeps `main` green: `sql-text`,
  `db-model`, `db-session`, then `db-ui` slice by slice (data sources and
  tree, console, grid, export and import). The first slice renders the tree
  from a model loaded by full passes only.
- The engine's `Introspector` grows; existing CLI commands keep working on
  the per-object methods.
- Console result tabs page less capably than table grids after another
  statement ran. This is the price of one cursor per connection; the offset
  fallback covers the common case. Observation of the reference (whether it asks
  for a row more than the page, whether next pages re-run the query) can
  change the choice for console results without touching the contracts.
- Thin Oracle sessions lose a manual transaction on cancel. The UI says so;
  the ODPI backend does not have the limit.
- Splitting `db-ui` later (explorer, console, grid as separate crates) is
  possible because every part talks to `db-session` only.
- Third-party code adapted for `sql-text` (statement splitting, formatter,
  completion helpers), import parsing and the SQL generators of the editing
  milestone comes from an MIT OR Apache-2.0 database client and needs one
  `notices/` entry per adapted piece (ADR 0019). Its interface code, theme,
  icons and texts are not used.
