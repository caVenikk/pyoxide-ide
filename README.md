# Pyoxide IDE

A fast, native IDE for Python, written in Rust.

Editor, debugger, test runner, version control and a built-in **database client**
in one lightweight app, with no JVM and no web view. Offline, open source, and
scriptable: every action is a command, so the UI, the CLI and AI agents drive
the same engine.

> **Status: pre-alpha.** Only the database client slice exists today
> (`pyoxide-db`). See the [roadmap](docs/ROADMAP.md) for what is planned and
> how far along it is.

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
**Overall parity: 8%** `██░░░░░░░░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 25% `██░░░░░░░░` | 2 | 3 | 10 |
| editor | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| settings-import | 0% `░░░░░░░░░░` | 0 | 0 | 4 |
| python-intel | 0% `░░░░░░░░░░` | 0 | 0 | 7 |
| run-debug | 0% `░░░░░░░░░░` | 0 | 0 | 4 |
| vcs | 0% `░░░░░░░░░░` | 0 | 0 | 3 |
| docker | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| platform | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| packaging | 30% `███░░░░░░░` | 0 | 1 | 1 |
| python-env | 0% `░░░░░░░░░░` | 0 | 0 | 9 |

Per-feature status: [docs/PROGRESS.md](docs/PROGRESS.md)
<!-- progress:end -->

## Workspace

| Crate | Purpose |
|---|---|
| `db-core` | Backend-agnostic traits, result model, schema introspection SQL |
| `db-oracle-thin` | Oracle over pure Rust (`oracle-rs`), no Oracle Client needed |
| `db-oracle-odpi` | Oracle through an installed Oracle Client (ODPI-C), fallback |
| `db-cli` | `pyoxide-db`, a command-line database client |

## Getting started

```bash
git clone <repository-url> && cd pyoxide-ide
docker compose up -d            # local Oracle Free for development (wait until healthy)
export PYOXIDE_DB_PASSWORD=pyoxide
cargo run -p db-cli -- --service FREEPDB1 --user pyoxide ping
cargo run -p db-cli -- --service FREEPDB1 --user pyoxide query "select * from dual"
cargo run -p db-cli -- --backend odpi --service FREEPDB1 --user pyoxide ping   # needs Oracle Client
```

The password is read only from `PYOXIDE_DB_PASSWORD`, never from arguments.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
PYOXIDE_TEST_ORACLE=1 cargo test -p db-oracle-thin -- --ignored   # needs docker compose up
```

## Documentation

- [Roadmap](docs/ROADMAP.md) · [Architecture](docs/architecture.md) · [Performance](docs/performance.md)
- [Clean-room rules](docs/clean-room.md) · [Theme formats](docs/theme-formats.md)
- [Contributing](CONTRIBUTING.md) · [Agent workflow](docs/agent-workflow.md) · [AGENTS.md](AGENTS.md)

## Built for agents

The repository is set up for AI-assisted development: [AGENTS.md](AGENTS.md)
holds the rules, `skills/` holds the development skill pack (see
[docs/agent-workflow.md](docs/agent-workflow.md)), and every user-visible action
is a registered command that agents can call.

## Contributing

Issues and pull requests are welcome; start with [CONTRIBUTING.md](CONTRIBUTING.md).
Please read the clean-room rules before sending code.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option (the usual Rust convention: you may use the code under either license).
Third-party notices are in [NOTICE](NOTICE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.

Pyoxide IDE is an independent project. It is not affiliated with, sponsored by
or endorsed by JetBrains s.r.o., the Python Software Foundation or Oracle
Corporation. PyCharm, IntelliJ, JetBrains, Python and Oracle are trademarks of
their respective owners and are used only to describe compatibility.
