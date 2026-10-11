# Architecture

## Principles
1. Rust everywhere in the application; external tools only as processes behind
   protocols (LSP, DAP, the Python interpreter).
2. Engine first: a pure-data model and a **command registry**; the UI is a thin
   layer. The UI, the CLI, a JSON control channel and agents call the same commands.
3. Nothing blocks the UI thread. Analysis and I/O are async, cancellable, incremental.
4. Traits at the boundaries: UI crates depend on traits (`db-core`), never on a
   concrete driver.

## Current crates
- `commands`: the command registry (ADR 0001).
- `db-core`: `Driver`, `Connection` and `Cursor` traits, typed values,
  cancel/timeout policy, errors, introspection model and per-dialect SQL,
  text table renderer (ADR 0002).
- `db-oracle-thin`: pure Rust Oracle driver (`oracle-rs`, TNS protocol).
- `db-oracle-odpi`: Oracle via an installed Oracle Client (ODPI-C), fallback for
  environments the thin driver cannot reach (Kerberos, LDAP naming, native
  network encryption, ...). Client libraries are loaded at runtime.
- `db-postgres`, `db-sqlite`: PostgreSQL and SQLite drivers (ADR 0003).
- `db-service`: connection profiles and secrets (ADR 0004), driver dispatch,
  diagnostics, `db.*` commands.
- `db-cli`: `pyoxide-db`, a thin front-end over `db-service`.

## Platform and products
One repository, one Cargo workspace: a language-agnostic platform layer and
one product layer per language, composed at compile time into one binary per
product (ADR 0005). Pyoxide is the Python product.

## Planned layers
`core` (project model, settings, commands) -> `services` (LSP/DAP clients, VCS,
Docker, DB, terminal) -> `ui` (toolkit chosen by ADR) -> `app`. Decisions are
recorded in [adr/](adr/).

## External processes
Python interpreter, ruff, a type checker, debugpy. Discovered and version-checked
at runtime; the IDE degrades gracefully when one is missing.
