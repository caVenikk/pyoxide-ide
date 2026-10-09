# Progress

_Generated from `replica/features.csv` by `scripts/progress.py`; do not edit by hand._

**Overall parity: 18%** `████░░░░░░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 48% `█████░░░░░` | 5 | 13 | 9 |
| db-gui | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| editor | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| settings-import | 0% `░░░░░░░░░░` | 0 | 0 | 4 |
| python-intel | 0% `░░░░░░░░░░` | 0 | 0 | 7 |
| run-debug | 0% `░░░░░░░░░░` | 0 | 0 | 4 |
| vcs | 0% `░░░░░░░░░░` | 0 | 0 | 3 |
| docker | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| platform | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| packaging | 30% `███░░░░░░░` | 0 | 1 | 1 |
| python-env | 0% `░░░░░░░░░░` | 0 | 0 | 9 |

✅ done · 🟡 partial · ⬜ not started. Features marked (own) are additions beyond the reference and are not part of the parity score.

## database

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Oracle connection (thin driver) | must | db-oracle-thin |
| ✅ | Oracle connection (Oracle Client / ODPI-C) | should | db-oracle-odpi: cancel, call timeouts, TNS/wallets/NNE/Kerberos; integration tests need Oracle Instant Client |
| ⬜ | PostgreSQL connection | must | MVP |
| ✅ | SQLite connection | must | db-sqlite (bundled SQLite) |
| 🟡 | Run SQL and view results | must | CLI and db.query command |
| 🟡 | Typed result values | must | SQLite and Oracle Client done; thin and PostgreSQL in progress |
| 🟡 | Schema introspection (tables / views / columns) | must | SQLite full; Oracle tables and columns |
| 🟡 | Indexes constraints sequences procedures | should | SQLite indexes, constraints, triggers |
| ✅ | Connection profiles with keychain secrets | must | TOML profiles, OS credential store, env, prompt |
| ⬜ | Password storage in KeePass (kdbx) | should | IDE password settings model |
| 🟡 | TLS and wallets | should | Oracle Client (tcps, wallets); diagnose TLS probe |
| 🟡 | Query timeout and cancel | must | SQLite, Oracle Client |
| 🟡 | Paginated results | must | cursor paging: SQLite, Oracle Client |
| ⬜ | Export results (CSV / JSON) | should |  |
| ✅ | Connection diagnostics (own) | should | pyoxide-db diagnose |
| ✅ | Test connection (server and driver version) | must | pyoxide-db test / db.connection.test |
| 🟡 | Bind parameters | should | SQLite, Oracle Client |
| 🟡 | Transaction mode (auto / manual) | should | SQLite, Oracle Client |
| 🟡 | Read-only connections | should | SQLite; Oracle Client best effort |
| 🟡 | Oracle TNS alias and connect descriptors | should | Oracle Client backend (dsn); thin in progress |
| ⬜ | Schema and object filters | should | Pro |
| 🟡 | Object source and DDL | should | SQLite done |
| ⬜ | SSH tunnel | should | Pro; after MVP; must never block the UI (feedback) |
| ⬜ | Startup script / keep-alive / auto-disconnect | could | Pro |
| 🟡 | Large values: capped fetch of LOB / JSON / long text | must | max_value_bytes and Value::Truncated; lazy full fetch todo |
| ⬜ | Persistent schema cache across restarts | should | feedback: introspection slow or forgotten |
| ⬜ | Cancellable scoped introspection with progress | should | feedback |
| ⬜ | PostgreSQL Unix-socket connections | could | feedback |
| ⬜ | External secret provider (command / password manager CLI) (own) | could | own feature; feedback |

## db-gui

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Database explorer tree | must | Pro |
| ⬜ | Data grid with paging | must | Pro; virtualized, renders visible cells only (feedback: grid freezes) |
| ⬜ | SQL console with history | should |  |
| ⬜ | Result table editing | could |  |
| ⬜ | Explain plan | could | Pro |
| ⬜ | Run a SQL file as a run configuration with results | should | feedback: top open request |
| ⬜ | Open a SQLite file directly | should | feedback |
| ⬜ | Date and number formats for grid, import and export | should | feedback |
| ⬜ | SQL*Plus commands in the Oracle console (DESCRIBE / EXEC) (own) | should | own feature; feedback |
| ⬜ | psql / sqlite3 meta-commands in the console (subset) (own) | could | own feature; feedback |

## editor

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Text buffer and editing | must |  |
| ⬜ | Python syntax highlighting | must |  |
| ⬜ | Tabs and split views | must |  |
| ⬜ | File tree and project model | must |  |
| ⬜ | Search in file and project | must |  |
| ⬜ | Multi-cursor and column selection | should |  |
| ⬜ | Code folding | should |  |
| ⬜ | Live templates | should |  |

## settings-import

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Keymap import | must |  |
| ⬜ | Code style import | should |  |
| ⬜ | Editor colour scheme import (.icls) | must |  |
| ⬜ | UI theme import (.theme.json) | must |  |
| ⬜ | Import data sources and passwords from PyCharm (own) | should | own feature; dataSources.xml + credential store |

## python-intel

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Interpreter and virtualenv discovery | must | venv uv poetry conda |
| ⬜ | Completion | must | LSP |
| ⬜ | Go to definition and find usages | must | LSP |
| ⬜ | Hover and signature help | must | LSP |
| ⬜ | Rename and refactorings | should |  |
| ⬜ | Lint and format (ruff) | must |  |
| ⬜ | Type checking | must | ty / basedpyright / pyrefly by ADR |

## run-debug

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Run configurations | must |  |
| ⬜ | Debugger (breakpoints / stepping / variables) | must | DAP / debugpy |
| ⬜ | Test runner (pytest) | should |  |
| ⬜ | Integrated terminal | must |  |

## vcs

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Git status and diff | must |  |
| ⬜ | Commit and branches | must |  |
| ⬜ | Blame and history | should |  |

## docker

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Docker containers and images | should |  |

## platform

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Command registry and palette (own) | must | registry and JSON channel; no palette yet |
| ⬜ | Plugin API (WASM) | could |  |

## packaging

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Windows / macOS / Linux builds | must | CI builds all three |
| ⬜ | Signed installers | should |  |

## python-env

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | uv: create virtualenv from the IDE | must | uv venv; choose Python version |
| ⬜ | uv: project init and sync (pyproject.toml / uv.lock) | must | uv init / uv sync / uv lock |
| ⬜ | uv: install and manage Python versions | should | uv python install |
| ⬜ | Package manager panel (list / install / upgrade / remove) | must | uv pip backend; pip fallback |
| ⬜ | Package search and version picker | should | PyPI index |
| ⬜ | Dependency groups and extras in pyproject.toml | should |  |
| ⬜ | uv run as run configuration | should |  |
| ⬜ | PEP 723 inline script dependencies | could |  |
| ⬜ | Other managers (poetry / conda / pipenv / pip) | should | after uv |
