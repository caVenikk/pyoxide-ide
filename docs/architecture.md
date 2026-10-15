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
- `search`: the text search engine of Find / Replace in the editor and of
  Find in Files: plain and regular expression queries with case and whole
  word options, replacement templates, cancellable searches over text
  snapshots off the UI thread.
- `syntax`: tree-sitter parsing off the UI thread and highlight spans as
  theme text roles; grammars and queries come from the products.
- `project`: an opened folder, its lazily listed file tree with hidden and
  excluded entries, Pyoxide's project folder (`.oxide/`) with the shared
  project settings (exclusions, and values by dotted table for products)
  and the one-time import of the reference IDE's exclusions (ADR 0008),
  the personal project state (ADR 0009), reading and writing documents.
- `project-search`: Find in Files without a toolkit: the files of a scope
  that are project content (the project's hidden and excluded entries left
  out), file masks, a parallel cancellable search that streams the matching
  lines and stops at a limit (later passes skip the lines shown), the
  results as ordered rows, and replacing in files on disk.
- `name-match`: the reference IDE's name matcher (camel humps, word
  starts, wildcards, middle matches) and its matching degree, for Go to
  File, actions and speed search.
- `file-index`: the project's files and folders by name, built in the
  background with the project tree's rules and kept current, and Go to
  File's matching and ranking (`name:line:column` included).
- `editor`: the code editor entity and element (virtualized painting,
  carets, selections, clipboard, input method, highlighting, editing
  commands) and its find bar (Find / Replace in the file); diagnostic
  marks, a link highlight, hover events and geometry for popups (stored
  and reported; painting comes with the problems in the editor).
- `code-intel`: the language-agnostic model and `CodeIntel` trait of code
  intelligence (ADR 0010): documents, positions as `text::Point`,
  diagnostics, completion items, documentation, signatures, locations,
  text and workspace edits, code actions, server status and events; an
  in-memory `FakeCodeIntel` (feature `fake`) for tests.
- `lsp-client`: the LSP client (ADR 0010): server specs, the product's
  `ServerCustomizer`, and `LspCodeIntel`, `CodeIntel` over several
  servers. A skeleton: servers are not started yet.
- `code-intel-ui`: the code intelligence of a main window: the product
  hook (`CodeIntelProvider`, `CodeIntelHook`), the `CodeIntelView` the
  workbench reports editors to, the `code.*` and `completion.*` commands.
  A skeleton: the commands do nothing yet.
- `themes`: theme model and tokens, default themes, import of PyCharm colour
  schemes and UI themes.
- `ui`: component layer on GPUI (ADR 0006, 0007): theme as GPUI colours and
  metrics, bundled fonts, colour icons rasterized at device scale, common
  components, context menus and list popups, the UI command index
  (`ui_commands!`, `command::invoke`).
- `workbench`: the main window (toolbar, stripes, tool windows, editor
  area, status bar, a movable and resizable popup frame, the Find in Files
  and Replace in Files popup with its preview, Search Everywhere and Go to
  File, Recent Files), the navigation history (Back and Forward),
  keymaps (with double taps of a modifier and the mouse's back and
  forward buttons), application start-up, environment switches and
  in-app measurements.

Python product (`crates/python/`):
- `py-env`: interpreter and virtualenv discovery, project model
  (`pyproject.toml`, `uv.lock`, PEP 723), uv and pip operations, `env.*`
  commands.
- `pyoxide-cli`: `pyoxide`, the product's command line over the platform and
  Python command registries.
- `py-syntax`: the Python grammar and highlight query for `syntax`.
- `py-lsp`: Python language services (ADR 0010, 0011): the tools (ruff,
  ty, basedpyright), their lookup modes, settings and server specs, the
  import of the reference's tool settings. A skeleton.
- `pyoxide-ide`: `pyoxide-ide`, the product's GUI application: the
  workbench with Python file types. Run it with
  `cargo run --release -p pyoxide-ide [folder] [files]`.

The application shell and editor are described in ADR 0007.

## Platform and products
One repository, one Cargo workspace: a language-agnostic platform layer and
one product layer per language, composed at compile time into one binary per
product (ADR 0005). Pyoxide is the Python product.

## Language services (M3, ADR 0010, 0011)
First version done (#82-#87): `lsp-client` (framing over `process`,
cancellation, document sync, position encodings, pull and push
diagnostics, routing per feature), `code-intel-ui` (completion,
documentation, parameter info, navigation, intention and usages popups,
the Language Services widget; `ui::markdown`), `editor` (painting of
diagnostics, error stripe, inspection widget, next / previous problem,
hover input) and `py-lsp` (ruff, ty and basedpyright: lookup, settings,
server configuration, the one-time import of the reference's tool
settings, ruff rule names and Disable actions, the type engine widget).
Later: Find Usages tool window, rename, Problems tool window, inlay hints,
semantic tokens, a file watcher.
- Layering (`scripts/check-layers.py`): `code-intel`, `lsp-client` and
  `py-lsp` know no UI toolkit; only `lsp-client` sees protocol types;
  platform UI crates depend on `code-intel`, never on `lsp-client`; the
  editor does not depend on `code-intel`.

## Planned layers
`core` (project model, settings, commands) -> `services` (LSP/DAP clients, VCS,
Docker, DB, terminal) -> `ui` (GPUI, ADR 0006) -> `app`. Decisions are recorded
in [adr/](adr/).

## External processes
Python interpreter, ruff, a type checker, debugpy. Discovered and version-checked
at runtime; the IDE degrades gracefully when one is missing.
