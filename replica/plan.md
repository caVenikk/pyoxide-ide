# Plan

Kept by the lead agent. Areas, dependencies, owners and status. Order of
areas: db (M1) -> editor (M2) -> python-env (M2b) -> python-intel (M3) ->
run-debug (M4) -> vcs (M5) -> db-gui (M6) -> docker, settings-import,
packaging (M7). Priorities are adjusted with `python3
skills/replica-diff/parity.py replica/features.csv` and `replica/perf.md`.

## Rules for parallel work
- Contracts first: traits, command ids, file formats are committed on `main`
  before sub-agents start.
- One owner per crate (or per file set). Sub-agents work in their own git
  worktree and branch, finish with `cargo fmt --all`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  measurements and a short report.
- Sub-agents do not edit `replica/features.csv`, `docs/PROGRESS.md`,
  `README.md` or the roadmap; the lead updates them when merging.
- The editor core and the UI toolkit are built by one agent, sequentially.

## M1: database layer (area db)

Contracts (lead, done first): `db-core` (values, cursor, cancel/timeout,
errors, introspection model), `commands` (registry), ADRs 0001-0004,
skeleton crates, `docker-compose.yml` (Oracle Free, PostgreSQL),
`docs/testing-databases.md`.

Work is tracked in GitHub issues (milestone "M1 Database layer"); commits
reference them with `Refs #N`, pull requests with `Closes #N`.

| issue | owner | crates / files | depends on | status |
|---|---|---|---|---|
| #1 PostgreSQL backend | sub-agent pg | `crates/platform/db-postgres`, `db-core/src/schema/postgres.rs` | contracts | merged |
| #2 SQLite backend | sub-agent sqlite | `crates/platform/db-sqlite`, `db-core/src/schema/sqlite.rs` | contracts | merged |
| #3 Oracle thin backend and introspection | sub-agent oracle-thin | `crates/platform/db-oracle-thin`, `db-core/src/schema/oracle.rs`, `db-core/src/oracle_net.rs` | contracts | merged |
| #4 Oracle Client backend | sub-agent oracle-odpi | `crates/platform/db-oracle-odpi` | contracts | merged (integration tests need Oracle Instant Client) |
| #5 db-service and CLI | lead | `crates/platform/db-service`, `crates/platform/db-cli`, `crates/platform/secrets` (store trait) | contracts | done |
| #6 diagnose | lead | `crates/platform/db-service/src/diagnose.rs`, `hints.rs`, `tls_probe.rs` | #5 | done |
| #7 KeePass store and PyCharm import | sub-agent secrets | `crates/platform/secrets` (KeePass, config, IDE store), `crates/platform/settings-import` | #5 store trait | merged (IDE credential naming to confirm) |
| #8 CI integration job and binaries | lead | `.github/workflows` | #1-#4 | written, runs after push |
| #9 Real Oracle 19c verification | maintainer | diagnose report | #3, #4, #6 | todo |
| #10 oracle-rs cancel upstream | open | upstream | #3 | todo |
| #17 Move crates into platform / product layout (ADR 0005) | lead | `crates/` | #16 done | done |
| #16 Deduplicate driver plumbing, conformance test kit | lead | `db-core` (worker, value helpers, Oracle SQL text, tx state), new `db-testkit` | #3 merged | done |

## M2: editor (started)

| issue | owner | crates / files | depends on | status |
|---|---|---|---|---|
| #11 Reference baseline | lead | `scripts/perf/`, `replica/perf.md` | - | startup, indexing, memory done; typing latency and large files open |
| #12 UI toolkit decision (spikes + ADR 0006) | sub-agent ui | `spikes/` (outside the workspace), `docs/adr/0006-ui-toolkit.md` | - | in progress (resumed) |
| #13 Control channel and headless UI tests | - | - | #12 | todo |
| #14 Theme import and Pyoxide theme model | sub-agent themes | `crates/platform/themes`, `docs/theme-mapping.md` | - | merged (importer; settings UI later) |
| #15 Python environments engine (uv) | sub-agent py-env | `crates/python/py-env`, `crates/python/pyoxide-cli` | - | merged (engine and CLI; UI later) |

Next after #12: application shell, text buffer, tree-sitter highlighting,
tabs, file tree; typing latency measured for the reference and Pyoxide.
