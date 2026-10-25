# Testing against real databases

Unit tests run everywhere with `cargo nextest run --workspace` (and
`cargo test --doc --workspace` for the doctests). SQLite tests always
run (in memory or in a temporary file). Tests that need a server are
`#[ignore]` and run on demand against the containers in `docker-compose.yml`.

```bash
docker compose up -d postgres oracle      # wait until both are healthy
docker compose ps                         # STATUS shows (healthy)
cargo test -p db-postgres -- --ignored
cargo test -p db-oracle-thin -- --ignored
cargo test -p db-oracle-odpi -- --ignored # needs an Oracle Client installed
cargo test -p db-service -- --ignored
cargo test -p db-session -- --ignored     # sessions: cancel, savepoints, read-only
```

| service | address | database | user | password |
|---|---|---|---|---|
| PostgreSQL 18 | `localhost:55432` | `pyoxide` | `pyoxide` | `$PYOXIDE_DB_PASSWORD`, default `pyoxide` |
| Oracle Database Free 23 | `localhost:1521` | service `FREEPDB1` | `pyoxide` | `$PYOXIDE_DB_PASSWORD`, default `pyoxide` |

The default password only protects throw-away local containers. Override any
value with `PYOXIDE_TEST_PG_HOST`, `PYOXIDE_TEST_PG_PORT`,
`PYOXIDE_TEST_ORACLE_HOST`, `PYOXIDE_TEST_ORACLE_PORT`,
`PYOXIDE_TEST_ORACLE_SERVICE`.

## Conformance suite
`crates/platform/db-testkit` holds one suite of contract cases that every driver
runs from its `tests/it/conformance.rs`:

| case | checks |
|---|---|
| `typed_round_trip`, `paging`, `busy_rule`, `early_close` | values and the one-cursor-per-connection rule |
| `cancel`, `timeout`, `cancel_while_paging` | stopping a statement, also while a cursor is open (skipped where the driver cannot) |
| `transactions`, `read_only`, `error_kinds` | commit and rollback, read-only sessions, error classification |
| `savepoint_round_trip` | `SAVEPOINT`, `ROLLBACK TO`, and `RELEASE` where the engine has it (Oracle has none) |
| `cursor_closed_by_next_statement` | a cursor is closed by the next statement on the connection, the rows already read stay valid |
| `bulk_load` | `Introspector::load` at every level returns what the per-object reads return, and the details the fixture makes |
| `load_query_count` | the statements of a load do not grow with the number of tables |

SQLite runs it in every `cargo test`; the server backends run it with the ignored tests:

```bash
cargo test -p db-postgres --test it conformance:: -- --ignored
cargo test -p db-oracle-thin --test it conformance:: -- --ignored
```

## Rules for integration tests
- Create what the test needs in the test (tables with a unique prefix per
  test) and drop it at the end; never depend on leftovers.
- A test must pass when run alone and in parallel with the others.
- Never print connection passwords; use `ConnectParams` and `Secret`.
- `Introspector::load` needs PostgreSQL 12 or later (`pg_attribute.attgenerated`); the
  per-object reads work on older servers. Oracle 19c is the oldest supported release
  (every dictionary view it reads is in 19c; the bulk load is not yet run on a
  real server, the Oracle conformance cases are the check).
- Oracle Free is version 23; behaviour specific to 19c (the oldest supported
  release) is checked with `pyoxide-db diagnose` on a real 19c server.

## Oracle Client for the ODPI backend
`db-oracle-odpi` loads Oracle Instant Client at runtime. Install Instant
Client Basic (or Basic Light) from Oracle's download page and make it
findable: on macOS set `DYLD_LIBRARY_PATH` or place the libraries next to the
binary, on Linux `LD_LIBRARY_PATH`, on Windows `PATH`. Without it, the ODPI
tests fail with `DPI-1047`, and `pyoxide-db diagnose` reports the backend as
unavailable.
