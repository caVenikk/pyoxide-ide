# Roadmap

Status legend: [x] done, [ ] planned. Milestones are developed per feature area
with the skill loop described in [agent-workflow.md](agent-workflow.md).

- [x] **M0** Workspace, `db-core`, Oracle thin and ODPI-C backends, `pyoxide-db` CLI
- [ ] **M1 Database layer**: typed values, full schema introspection, connection
      profiles (TOML, secrets in keychain), TLS and wallets, timeouts and
      cancellation, pagination, `diagnose` command, integration tests against
      containers. **MVP database clients: Oracle, PostgreSQL and SQLite** behind
      the same `db-core` traits (new crates `db-postgres`, `db-sqlite`); other
      engines (MySQL, SQL Server, ...) come after the MVP
- [ ] **M2 Editor core**: text buffer, tree-sitter Python highlighting, UI toolkit
      decision (ADR + measurements), tabs, file tree, project and virtualenv
      discovery (venv, uv, poetry, conda)
- [ ] **M2b Python environments (uv-first)**: native uv integration: create
      virtualenvs, `uv init/sync/lock`, Python version management, package manager
      panel (list, install, upgrade, remove, search) on top of uv with a pip
      fallback, `uv run` run configurations; poetry/conda/pipenv afterwards
- [ ] **M3 Python intelligence**: LSP client; ruff (lint, format); type checker
      choice (ty, basedpyright or pyrefly) by ADR; completion, hover, go to
      definition, find usages, rename, code actions
- [ ] **M4 Run and debug**: run configurations, pytest runner with result tree,
      terminal, DAP client for debugpy
- [ ] **M5 Version control**: git status, diff, blame, commit UI
- [ ] **M6 Database GUI**: schema tree, SQL console, data grid, export
- [ ] **M7 Integrations**: Docker, settings import (keymaps, code styles, live
      templates, colour schemes `.icls`, UI themes `.theme.json`; data files
      only), packaging for macOS/Windows/Linux, WASM plugin API

Priorities are adjusted from the feature parity report (`replica/features.csv`)
and the performance report (`replica/perf.md`), not the other way around.
