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
| #1 PostgreSQL backend | sub-agent pg | `crates/db-postgres`, `db-core/src/schema/postgres.rs` | contracts | merged |
| #2 SQLite backend | sub-agent sqlite | `crates/db-sqlite`, `db-core/src/schema/sqlite.rs` | contracts | merged |
| #3 Oracle thin backend and introspection | sub-agent oracle-thin | `crates/db-oracle-thin`, `db-core/src/schema/oracle.rs`, `db-core/src/oracle_net.rs` | contracts | in progress |
| #4 Oracle Client backend | sub-agent oracle-odpi | `crates/db-oracle-odpi` | contracts | merged (integration tests need Oracle Instant Client) |
| #5 db-service and CLI | lead | `crates/db-service`, `crates/db-cli`, `crates/secrets` (store trait) | contracts | done |
| #6 diagnose | lead | `crates/db-service/src/diagnose.rs`, `hints.rs`, `tls_probe.rs` | #5 | done |
| #7 KeePass store and PyCharm import | sub-agent secrets | `crates/secrets` (KeePass, config, IDE store), `crates/settings-import` | #5 store trait | merged (IDE credential naming to confirm) |
| #8 CI integration job and binaries | lead | `.github/workflows` | #1-#4 | written, runs after push |
| #9 Real Oracle 19c verification | maintainer | diagnose report | #3, #4, #6 | todo |
| #10 oracle-rs cancel upstream | open | upstream | #3 | todo |
| #17 Move crates into platform / product layout (ADR 0005) | lead | `crates/` | #16 done | after #16, on request |
| #16 Deduplicate driver plumbing, conformance test kit | lead | `db-core` (worker, value helpers, Oracle SQL text, tx state), new `db-testkit` | #3 merged | next, starts on request |

## Next areas (not started)
- M2 editor: #11 reference baseline (first), #12 UI toolkit ADR with measured
  spikes, #13 control channel and headless UI tests; then text buffer and
  highlighting.
- M2b python-env: #15 uv integration.
- Themes: #14 `.icls` / `.theme.json` importer and `docs/theme-mapping.md`
  (can run in parallel with M2 once the theme token format exists).
