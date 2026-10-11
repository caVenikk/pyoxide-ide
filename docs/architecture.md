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
Platform (`crates/platform/`, language-agnostic):
- `commands`: the command registry (ADR 0001).
- `db-core`: `Driver`, `Connection` and `Cursor` traits, typed values,
  cancel/timeout policy, errors, introspection model and per-dialect SQL,
  shared driver building blocks (blocking-driver worker, value conversion,
  Oracle statement text, transaction state), text table renderer (ADR 0002).
- `db-oracle-thin`: pure Rust Oracle driver (`oracle-rs`, TNS protocol).
- `db-oracle-odpi`: Oracle via an installed Oracle Client (ODPI-C), fallback for
  environments the thin driver cannot reach (Kerberos, LDAP naming, native
  network encryption, ...). Client libraries are loaded at runtime.
- `db-postgres`, `db-sqlite`: PostgreSQL and SQLite drivers (ADR 0003).
- `db-testkit`: the conformance suite every database driver runs.
- `db-service`: connection profiles (ADR 0004), driver dispatch, diagnostics,
  import of PyCharm data sources, `db.*` commands.
- `db-cli`: `pyoxide-db`, a thin front-end over `db-service`.
- `secrets`: secret stores (OS credential store, KeePass, memory), their
  settings and the reader of a JetBrains IDE credential store for import.
- `settings-import`: PyCharm project settings (data sources, JDBC URLs).
- `text`: the text model (ADR 0007): rope buffer, selections, undoable
  transactions, caret movement, encoding and line separator of the file.
- `syntax`: tree-sitter parsing off the UI thread and highlight spans as
  theme text roles; grammars and queries come from the products.
- `project`: an opened folder, its lazily listed file tree with ignore
  rules, reading and writing documents.
- `editor`: the code editor entity and element (virtualized painting,
  carets, selections, clipboard, input method, highlighting, editing
  commands).
- `themes`: theme model and tokens, default themes, import of PyCharm colour
  schemes and UI themes.
- `ui`: component layer on GPUI (ADR 0006, 0007): theme as GPUI colours and
  metrics, bundled fonts, colour icons rasterized at device scale, common
  components, the UI command index (`ui_commands!`, `command::invoke`).
- `workbench`: the main window (toolbar, stripes, tool windows, editor
  area, status bar), keymaps, application start-up, environment switches
  and in-app measurements.

Python product (`crates/python/`):
- `py-env`: interpreter and virtualenv discovery, project model
  (`pyproject.toml`, `uv.lock`, PEP 723), uv and pip operations, `env.*`
  commands.
- `pyoxide-cli`: `pyoxide`, the product's command line over the platform and
  Python command registries.
- `py-syntax`: the Python grammar and highlight query for `syntax`.
- `pyoxide-ide`: `pyoxide-ide`, the product's GUI application: the
  workbench with Python file types. Run it with
  `cargo run --release -p pyoxide-ide [folder] [files]`.

The application shell and editor are described in ADR 0007.

## Platform and products
One repository, one Cargo workspace: a language-agnostic platform layer and
one product layer per language, composed at compile time into one binary per
product (ADR 0005). Pyoxide is the Python product.

## Planned layers
`core` (project model, settings, commands) -> `services` (LSP/DAP clients, VCS,
Docker, DB, terminal) -> `ui` (GPUI, ADR 0006) -> `app`. Decisions are recorded
in [adr/](adr/).

## External processes
Python interpreter, ruff, a type checker, debugpy. Discovered and version-checked
at runtime; the IDE degrades gracefully when one is missing.
