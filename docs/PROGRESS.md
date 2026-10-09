# Progress

_Generated from `replica/features.csv` by `scripts/progress.py`; do not edit by hand._

**Overall parity: 17%** `███░░░░░░░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 50% `█████░░░░░` | 5 | 10 | 8 |
| db-gui | 0% `░░░░░░░░░░` | 0 | 0 | 5 |
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
| ✅ | Oracle connection (Oracle Client / ODPI-C) | should | db-oracle-odpi |
| ⬜ | PostgreSQL connection | must | MVP |
| ✅ | SQLite connection | must | db-sqlite (bundled SQLite) |
| 🟡 | Run SQL and view results | must | CLI and db.query command |
| 🟡 | Typed result values | must | SQLite done; Oracle and PostgreSQL in progress |
| 🟡 | Schema introspection (tables / views / columns) | must | SQLite full; Oracle tables and columns |
| 🟡 | Indexes constraints sequences procedures | should | SQLite indexes, constraints, triggers |
| ✅ | Connection profiles with keychain secrets | must | TOML profiles, OS credential store, env, prompt |
| ⬜ | Password storage in KeePass (kdbx) | should | IDE password settings model |
| ⬜ | TLS and wallets | should |  |
| 🟡 | Query timeout and cancel | must | SQLite done |
| 🟡 | Paginated results | must | cursor paging: SQLite done |
| ⬜ | Export results (CSV / JSON) | should |  |
| ✅ | Connection diagnostics (own) | should | pyoxide-db diagnose |
| ✅ | Test connection (server and driver version) | must | pyoxide-db test / db.connection.test |
| 🟡 | Bind parameters | should | SQLite done |
| 🟡 | Transaction mode (auto / manual) | should | SQLite done |
| 🟡 | Read-only connections | should | SQLite done |
| ⬜ | Oracle TNS alias and connect descriptors | should | Oracle Client backend |
| ⬜ | Schema and object filters | could | Pro |
| 🟡 | Object source and DDL | should | SQLite done |
| ⬜ | SSH tunnel | could | Pro; after MVP |
| ⬜ | Startup script / keep-alive / auto-disconnect | could | Pro |

## db-gui

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Database explorer tree | must | Pro |
| ⬜ | Data grid with paging | must | Pro |
| ⬜ | SQL console with history | should |  |
| ⬜ | Result table editing | could |  |
| ⬜ | Explain plan | could | Pro |

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
