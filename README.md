# Pyoxide IDE

A fast, native IDE for Python, written in Rust.

Editor, debugger, test runner, version control and a built-in **database client**
in one lightweight app, with no JVM and no web view. Offline, open source, and
scriptable: every action is a command, so the UI, the CLI and AI agents drive
the same engine.

> **Status: pre-alpha.** The first editor window exists: open a folder, browse
> the project tree, edit Python with highlighting and the familiar keymap, save
> (`cargo run --release -p pyoxide-ide -- <folder>`). No search, completion,
> run or debug yet. The engines are usable from the command line: the database
> client (`pyoxide-db`, Oracle, PostgreSQL, SQLite), Python environments on top
> of uv (`pyoxide env`) and import of PyCharm colour schemes, UI themes and data
> sources. See the [roadmap](docs/ROADMAP.md) for what is planned and how far
> along it is.

## Goals

- **Speed first.** Native Rust, GPU rendering, everything off the UI thread.
  Performance budgets are tracked in CI ([docs/performance.md](docs/performance.md)).
- **Python only.** No attempt to be a universal IDE; fewer moving parts.
- **Standing on good tools.** Python intelligence comes from existing Rust/LSP
  tooling (ruff, a type checker such as ty, basedpyright or pyrefly) and
  debugging from debugpy over DAP, instead of a hand-written analyzer.
- **Windows is a first-class platform**, together with macOS and Linux.
- **Familiar to PyCharm users**: import of keymaps, code styles, live
  templates, editor colour schemes (`.icls`) and UI themes (`.theme.json`).
- **Clean-room.** Built from public documentation, open specifications,
  permissively licensed code and observed behaviour only. See
  [docs/clean-room.md](docs/clean-room.md).

## Progress

<!-- progress:start -->
**Overall parity: 35%** `███████░░░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 71% `███████░░░` | 15 | 6 | 6 |
| db-gui | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| editor | 44% `████░░░░░░` | 1 | 6 | 3 |
| settings-import | 27% `███░░░░░░░` | 0 | 2 | 2 |
| python-intel | 3% `░░░░░░░░░░` | 0 | 1 | 20 |
| run-debug | 21% `██░░░░░░░░` | 1 | 0 | 4 |
| vcs | 0% `░░░░░░░░░░` | 0 | 0 | 3 |
| docker | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| platform | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| packaging | 30% `███░░░░░░░` | 0 | 1 | 1 |
| python-env | 50% `█████░░░░░` | 0 | 9 | 0 |

Per-feature status: [docs/PROGRESS.md](docs/PROGRESS.md)
<!-- progress:end -->

## Workspace

Crates live in `crates/platform/` (language-agnostic) and `crates/python/`
(the Python product); see ADR 0005.

| Crate | Purpose |
|---|---|
| `commands` | Command registry shared by the UI, the CLI and the agent control channel |
| `db-core` | Backend-agnostic traits, typed values, cursors, cancel/timeouts, schema introspection |
| `db-oracle-thin` | Oracle over pure Rust (`oracle-rs`), no Oracle Client needed |
| `db-oracle-odpi` | Oracle through an installed Oracle Client (ODPI-C), fallback |
| `db-postgres` | PostgreSQL (`tokio-postgres`, rustls) |
| `db-sqlite` | SQLite (`rusqlite`, bundled) |
| `db-service` | Connection profiles, diagnostics, PyCharm import, `db.*` commands |
| `db-testkit` | Conformance suite every database driver runs |
| `secrets` | Secret stores: OS credential store, KeePass, memory |
| `settings-import` | Import of PyCharm project settings (data sources) |
| `themes` | Theme model, default themes, import of PyCharm colour schemes and UI themes |
| `text` | Rope text buffer, selections, undo/redo, line endings and encodings |
| `syntax` | tree-sitter parsing and highlighting off the UI thread |
| `project` | Project folder, lazy file tree, reading and saving documents |
| `ui` | Component layer on GPUI: theme, fonts, icons, UI commands |
| `editor` | The code editor view: layout, carets, selections, input, editing commands |
| `workbench` | Main window: toolbar, tool windows, tabs, status bar, keymaps |
| `py-syntax` (python) | Python grammar, highlight query and editing rules |
| `pyoxide-ide` (python) | `pyoxide-ide`, the Python product's application |
| `py-env` (python) | Interpreters, virtualenvs, uv and pip integration, `env.*` commands |
| `pyoxide-cli` (python) | `pyoxide`, the Python product's command line (`env`, plus all `db.*` commands) |
| `db-cli` | `pyoxide-db`, a command-line database client |

## Getting started

The IDE window (the default keymaps and what works so far:
[tests/scenarios/editor/keymap-and-editing-basics.md](tests/scenarios/editor/keymap-and-editing-basics.md)):

```bash
git clone https://github.com/caVenikk/pyoxide-ide && cd pyoxide-ide
cargo run --release -p pyoxide-ide -- path/to/project
```

The database client:

```bash
docker compose up -d            # PostgreSQL 18 and Oracle Free 23 for development
export PYOXIDE_DB_PASSWORD=pyoxide
cargo run -p db-cli -- --service FREEPDB1 --user pyoxide test
cargo run -p db-cli -- --backend postgres --port 55432 --database pyoxide --user pyoxide query "select now()"
cargo run -p db-cli -- --path :memory: query "select sqlite_version()"
cargo run -p db-cli -- --service FREEPDB1 --user pyoxide diagnose
```

Passwords come from the environment, the OS credential store or a prompt,
never from arguments. Profiles, secrets and diagnostics:
[docs/pyoxide-db.md](docs/pyoxide-db.md).

Python environments (needs [uv](https://docs.astral.sh/uv/); details in
[docs/python-environments.md](docs/python-environments.md)):

```bash
cargo run -p pyoxide-cli -- env interpreters
cargo run -p pyoxide-cli -- env info .
cargo run -p pyoxide-cli -- commands
```

Import report for a PyCharm colour scheme, UI theme or theme plugin:

```bash
cargo run -p themes --example import -- path/to/theme.jar
```

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p db-postgres -p db-oracle-thin -- --ignored   # needs docker compose up
```

## Documentation

- [Roadmap](docs/ROADMAP.md) · [Architecture](docs/architecture.md) · [Performance](docs/performance.md)
- [pyoxide-db](docs/pyoxide-db.md) · [Testing against databases](docs/testing-databases.md) · [Decisions (ADR)](docs/adr/)
- [Clean-room rules](docs/clean-room.md) · [Theme formats](docs/theme-formats.md)
- [Contributing](CONTRIBUTING.md) · [Agent workflow](docs/agent-workflow.md) · [AGENTS.md](AGENTS.md)

## Built for agents

The repository is set up for AI-assisted development: [AGENTS.md](AGENTS.md)
holds the rules, `skills/` holds the development skill pack (see
[docs/agent-workflow.md](docs/agent-workflow.md)), and every user-visible action
is a registered command that agents can call.

## Contributing

Issues and pull requests follow [CONTRIBUTING.md](CONTRIBUTING.md) and the
[clean-room rules](docs/clean-room.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at the licensee's option. This is the common licensing of the Rust ecosystem:
Apache-2.0 carries an explicit patent grant, MIT keeps compatibility with
GPL-2.0 projects. Contributions submitted for inclusion are licensed under the
same terms, without additional conditions. Third-party notices are in
[NOTICE](NOTICE).

## Trademarks

Pyoxide IDE is an independent project, not affiliated with, sponsored by or
endorsed by JetBrains s.r.o., the Python Software Foundation or Oracle
Corporation. PyCharm, IntelliJ, JetBrains, Python and Oracle are trademarks of
their respective owners and are used only to describe compatibility.
