# Progress

_Generated from `replica/features.csv` by `scripts/progress.py`; do not edit by hand._

**Overall parity: 38%** `████████░░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 71% `███████░░░` | 15 | 6 | 6 |
| db-gui | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| editor | 38% `████░░░░░░` | 1 | 4 | 4 |
| settings-import | 27% `███░░░░░░░` | 0 | 2 | 2 |
| python-intel | 8% `█░░░░░░░░░` | 0 | 1 | 6 |
| run-debug | 0% `░░░░░░░░░░` | 0 | 0 | 4 |
| vcs | 0% `░░░░░░░░░░` | 0 | 0 | 3 |
| docker | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| platform | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| packaging | 30% `███░░░░░░░` | 0 | 1 | 1 |
| python-env | 45% `████░░░░░░` | 0 | 8 | 1 |

✅ done · 🟡 partial · ⬜ not started. Features marked (own) are additions beyond the reference and are not part of the parity score.

## database

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Oracle connection (thin driver) | must | db-oracle-thin over oracle-rs with a PL/SQL wrapper (works around oracle-rs bugs); no NNE, no cancel |
| ✅ | Oracle connection (Oracle Client / ODPI-C) | should | db-oracle-odpi: cancel, call timeouts, TNS/wallets/NNE/Kerberos; integration tests need Oracle Instant Client |
| ✅ | PostgreSQL connection | must | db-postgres (tokio-postgres, rustls) |
| ✅ | SQLite connection | must | db-sqlite (bundled SQLite) |
| 🟡 | Run SQL and view results | must | CLI and db.query command |
| ✅ | Typed result values | must | all four backends |
| ✅ | Schema introspection (tables / views / columns) | must | Oracle (ALL_* views, 19c-safe), PostgreSQL, SQLite |
| ✅ | Indexes constraints sequences procedures | should | Oracle, PostgreSQL, SQLite |
| ✅ | Connection profiles with keychain secrets | must | TOML profiles, OS credential store, env, prompt |
| ✅ | Password storage in KeePass (kdbx) | should | secrets: KDBX 4 store, master password in OS store / env / prompt |
| 🟡 | TLS and wallets | should | PostgreSQL all modes; Oracle Client tcps and wallets; thin: TCPS with ewallet.pem, no cwallet.sso; TCPS untested live |
| 🟡 | Query timeout and cancel | must | timeouts everywhere; cancel on SQLite, PostgreSQL, Oracle Client (not thin) |
| ✅ | Paginated results | must | incremental cursors on all backends |
| ⬜ | Export results (CSV / JSON) | should |  |
| ✅ | Connection diagnostics (own) | should | pyoxide-db diagnose |
| ✅ | Test connection (server and driver version) | must | pyoxide-db test / db.connection.test |
| ✅ | Bind parameters | should | all four backends |
| ✅ | Transaction mode (auto / manual) | should | all four backends |
| 🟡 | Read-only connections | should | SQLite, PostgreSQL; Oracle client-side best effort |
| ✅ | Oracle TNS alias and connect descriptors | should | Oracle Client (dsn as is); thin via oracle_net (Easy Connect, descriptors, tnsnames.ora) |
| ⬜ | Schema and object filters | should | Pro |
| ✅ | Object source and DDL | should | Oracle (ALL_SOURCE, DBMS_METADATA), PostgreSQL, SQLite |
| ⬜ | SSH tunnel | should | Pro; after MVP; must never block the UI (feedback) |
| ⬜ | Startup script / keep-alive / auto-disconnect | could | Pro |
| 🟡 | Large values: capped fetch of LOB / JSON / long text | must | max_value_bytes and Value::Truncated; lazy full fetch todo |
| ⬜ | Persistent schema cache across restarts | should | feedback: introspection slow or forgotten |
| ⬜ | Cancellable scoped introspection with progress | should | feedback |
| 🟡 | PostgreSQL Unix-socket connections | could | implemented, untested |
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
| 🟡 | Text buffer and editing | must | rope buffer, undo/redo, clipboard, IME, default keymaps and editing basics (pairing, smart indent, comment, line moves, extend selection, go to line); no reload of files changed on disk |
| 🟡 | Main window chrome (toolbar, tool-window stripes, status bar) | must | New UI layout and Islands themes; toolbar file path; stripe buttons, run widget and status bar widgets are placeholders; native caption on Windows/Linux |
| ✅ | Python syntax highlighting | must | tree-sitter off the UI thread mapped to theme text roles |
| 🟡 | Tabs and split views | must | tabs with modified marker and keyboard switching; no split views |
| 🟡 | File tree and project model | must | open folder, lazy tree with ignore rules, keyboard navigation; no file operations or drag and drop |
| ⬜ | Search in file and project | must |  |
| ⬜ | Multi-cursor and column selection | should |  |
| ⬜ | Code folding | should |  |
| ⬜ | Live templates | should |  |

## settings-import

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Keymap import | must |  |
| ⬜ | Code style import | should |  |
| 🟡 | Editor colour scheme import (.icls) | must | themes crate: schemes with parents and baseAttributes, report; no settings UI yet |
| 🟡 | UI theme import (.theme.json) | must | themes crate: theme.json and plugin jars (data files only), report; no settings UI yet |
| ✅ | Import data sources and passwords from PyCharm (own) | should | own feature; verified with the macOS keychain; KeePass and Windows DPAPI paths tested on synthetic data |

## python-intel

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Interpreter and virtualenv discovery | must | py-env: uv, PATH, venv, Poetry, conda, pyenv, py launcher; no UI yet |
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
| 🟡 | uv: create virtualenv from the IDE | must | engine and pyoxide CLI (env venv); no UI yet |
| 🟡 | uv: project init and sync (pyproject.toml / uv.lock) | must | engine and CLI (env init/sync/lock); no UI yet |
| 🟡 | uv: install and manage Python versions | should | engine and CLI (env python-install); no UI yet |
| 🟡 | Package manager panel (list / install / upgrade / remove) | must | engine and CLI (env packages/add/remove, outdated); panel UI todo |
| 🟡 | Package search and version picker | should | PyPI lookup (env search); picker UI todo |
| 🟡 | Dependency groups and extras in pyproject.toml | should | read and edit via uv add --group/--optional; no UI yet |
| ⬜ | uv run as run configuration | should |  |
| 🟡 | PEP 723 inline script dependencies | could | parsed by the project model |
| 🟡 | Other managers (poetry / conda / pipenv / pip) | should | discovery of Poetry/conda/Pipenv, pip fallback for packages |
