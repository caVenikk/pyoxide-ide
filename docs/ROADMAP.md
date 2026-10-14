# Roadmap

Status legend: [x] done, [ ] planned. Milestones are developed per feature area
with the skill loop described in [agent-workflow.md](agent-workflow.md).

- [x] **M0** Workspace, `db-core`, Oracle thin and ODPI-C backends, `pyoxide-db` CLI
- [x] **M1 Database layer**: typed values, full schema introspection, connection
      profiles (TOML, secrets in keychain), TLS and wallets, timeouts and
      cancellation, pagination, `diagnose` command, integration tests against
      containers. **MVP database clients: Oracle, PostgreSQL and SQLite** behind
      the same `db-core` traits (new crates `db-postgres`, `db-sqlite`); other
      engines (MySQL, SQL Server, ...) come after the MVP. Done; open checks:
      Oracle Client integration tests (#4), a real Oracle 19c server (#9),
      statement cancel in `oracle-rs` upstream (#10)
- [x] **M2 Editor core**: text buffer, tree-sitter Python highlighting, UI toolkit
      decision (ADR + measurements), tabs, file tree, project and virtualenv
      discovery (venv, uv, poetry, conda). Done: GPUI (ADR 0006), application
      shell and editor (ADR 0007, #18) within the typing and large-file budgets
      (replica/perf.md), interpreter discovery (`py-env`). Next for the editor:
      find and replace, Go to File, split views, multiple carets, folding
- [ ] **M2b Python environments (uv-first)**: native uv integration: create
      virtualenvs, `uv init/sync/lock`, Python version management, package manager
      panel (list, install, upgrade, remove, search) on top of uv with a pip
      fallback, `uv run` run configurations; poetry/conda/pipenv afterwards.
      In progress: engine and `pyoxide env` command line done; panels need the
      application shell (#15)
- [ ] **MVP** (milestone "MVP", built first): the first version that is usable
      for daily Python work and is announced to the open-source community.
      Python intelligence first version (M3: ruff and ty, problems in the
      editor, completion, documentation, parameter info, go to declaration
      and usages, Alt+Enter, reformat; ADR 0010, 0011), files reloaded when
      they change on disk (#36), file operations in the project tree (#37),
      run configurations (#74), configured interpreters (#75), terminal
      (#89), debugger with debugpy (#90), basic git (#91, #28), a settings
      window with the basic pages (#92, #40), the rebrand (#93) and release
      builds for Windows (per-user and per-machine installer, portable zip),
      macOS and Linux (#94). The rest of M3-M7 follows the MVP.
- [ ] **M3 Python intelligence**: LSP client; ruff (lint, format); ty as the
      type server with basedpyright as an option (ADR 0011); completion, hover,
      go to definition, find usages, rename, code actions
- [ ] **M4 Run and debug**: run configurations, pytest runner with result tree,
      terminal, DAP client for debugpy
- [ ] **M5 Version control**: git status, diff, blame, commit UI
- [ ] **M6 Database GUI**: schema tree, SQL console, data grid, export
- [ ] **M7 Integrations**: Docker, settings import (keymaps, code styles, live
      templates, colour schemes `.icls`, UI themes `.theme.json`; data files
      only), packaging for macOS/Windows/Linux, WASM plugin API. Colour scheme
      and UI theme import done in the `themes` crate (settings UI pending);
      PyCharm data sources and passwords import done

Priorities are adjusted from the feature parity report (`replica/features.csv`)
and the performance report (`replica/perf.md`), not the other way around.
