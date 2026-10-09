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

| work package | owner | crates / files | depends on | status |
|---|---|---|---|---|
| WP1 PostgreSQL backend | sub-agent pg | `crates/db-postgres`, `db-core/src/schema/postgres.rs` | contracts | todo |
| WP2 SQLite backend | sub-agent sqlite | `crates/db-sqlite`, `db-core/src/schema/sqlite.rs` | contracts | todo |
| WP3 Oracle backends | sub-agent oracle | `crates/db-oracle-thin`, `crates/db-oracle-odpi`, `db-core/src/schema/oracle.rs` | contracts | todo |
| WP4 Service and CLI | lead | `crates/db-service`, `crates/db-cli` | contracts | todo |
| WP5 Integration, CI, measurements | lead | `.github/workflows`, `replica/perf.md` | WP1-WP4 | todo |

WP4 scope: profiles (TOML), secrets (env, OS keychain, prompt), driver
dispatch, `db.*` commands, `pyoxide-db diagnose`, `--share` redaction,
`db-bench` example.

## Next areas (not started)
- M2 editor: UI toolkit ADR with measured spikes, text buffer, highlighting.
  Needs the reference baseline in `replica/perf.md` first.
- M2b python-env: uv integration.
- Themes: `.icls` / `.theme.json` importer and `docs/theme-mapping.md`
  (settings-import area, can run in parallel with M2 once the theme token
  format exists).
