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
| #12 UI toolkit decision (prototypes + ADR 0006) | sub-agent ui | `docs/adr/0006-ui-toolkit.md` | - | done: GPUI (ADR 0006); prototypes not kept |
| #13 Control channel and headless UI tests | - | - | #12 | todo |
| #14 Theme import and Pyoxide theme model | sub-agent themes | `crates/platform/themes`, `docs/theme-mapping.md` | - | merged (importer; settings UI later) |
| #15 Python environments engine (uv) | sub-agent py-env | `crates/python/py-env`, `crates/python/pyoxide-cli` | - | merged (engine and CLI; UI later) |

| #18 Application shell and editor MVP on GPUI | sub-agent shell | `crates/platform/{text,syntax,project,ui,editor,workbench}`, `crates/python/{py-syntax,pyoxide-ide}`, ADR 0007 | #12 | done (PR #22) |
| #31 Find and replace in file | sub-agent find | new `crates/platform/search`, `editor` (find bar, highlights), `ui` (field components), `themes` (search roles), keymaps | #18 | done (PR #46) |
| #47 Tree scrolls with the wheel and trackpad | sub-agent scroll | `workbench` (project panel), `editor` (one-line fields) | #18 | done (PR #54) |
| #61 Excluded folders and ignored files | sub-agent exclusions | `project` (filter, `.oxide` store, `.idea` import), `ui` (context menu), `workbench` (tree), `themes`, ADR 0008 | #18 | done (PR #62) |
| #43 Find in Files | sub-agent find | new `crates/platform/project-search`, `workbench` (popup frame, find in files), `ui` | #31, #61 | done (PR #63) |
| #64 Drop files onto the window to open them | sub-agent drop | `workbench` (editor area, empty state) | #18 | done (PR #70) |
| #71 SQLite lock waits starved by back-to-back commits | sub-agent sqlite | `db-sqlite` | - | done (PR #72) |
| #44 Go to File and Search Everywhere | sub-agent find | `ui` (popup, list, fuzzy), `project` (file name index), `workbench` | #43 | done (PR #73) |
| #45 Run the current Python file | sub-agents run (A1 non-UI, A2 UI, B interpreter) | new `crates/platform/{process,console}`, `crates/python/py-run`; `workbench` (bottom tool window area, Run tool window, run widget, dialogs, product hooks), `editor` (console mode), `project` (personal state, ADR 0009), `ui` (list popups), `pyoxide-ide` (Python run provider, interpreter widget) | #18 | done (PR #76) |
| #74 Run configurations with a one-time import from `.idea` | sub-agent run-config | new `crates/platform/run-config`, `workbench/src/run/**`, `ui` (menu), `py-run`, `pyoxide-ide`, ADR 0012 | #45 | done (PR #117) |
| #75 Configured interpreters, Add New Interpreter, interpreter settings | - | python-env | #45 | todo |

## M3: Python intelligence (recon done)

Recon: `replica/recon.md` (area python-intel), scenarios in
`tests/scenarios/python-intel/`, ADR 0010 (language services over LSP) and
ADR 0011 (type server: ty by default, basedpyright as an option), both
accepted. Defaults: ruff and ty on (Pyoxide has no engine of its own). Milestone "M3 Python
intelligence".

| issue | owner | crates / files | depends on | status |
|---|---|---|---|---|
| #82 Contracts: model, crate skeletons, ADRs | sub-agent contracts, reviewed by the lead | new `code-intel`, skeletons `lsp-client`, `code-intel-ui`, `py-lsp`; `text` (`AppliedEdit`), `editor` stubs, `workbench` (`Product.code_intel`, `FileType.language_id`), `project` store values, `py-env` `env_tool`, icons, workspace deps | - | done (PR #88) |
| #83 LSP client | sub-agent lsp | `crates/platform/lsp-client`, `code-intel` impl parts, fake server binary | #82 | done (PR #95) |
| #84 Problems in the editor, hover input | sub-agent editor | `crates/platform/editor` (`editor.rs`, `element.rs`, new `diagnostics.rs`), F2 bindings | #82 | done (PR #98) |
| #85 Completion, controller, Language Services widget | sub-agent intel-ui | `crates/platform/code-intel-ui`, `workbench.rs` integration hunks, keymaps, `tests/keymap.rs` | #82, events of #84 | done (PR #100) |
| #86 Quick doc, parameter info, navigation, intentions, reformat | sub-agent intel-ui-2 | `code-intel-ui` (own modules), `ui/src/markdown.rs` | #85 first checkpoint | done (PR #102) |
| #87 Python services: ruff, ty, basedpyright option | sub-agent py-lsp | `crates/python/py-lsp`, `pyoxide-ide` (provider, type engine widget, ruff intentions, interpreter observer) | #82, #83 for live runs | done (PR #99) |

Order: #82 on `main`, then #83, #84, #85, #87 in parallel, #86 from #85's
first checkpoint. Merge order #84, #83, #87, #85, #86 (the integration and
keymaps rebase last). Later in M3: Find Usages tool window, rename,
highlight usages, Problems tool window, Go to Class / Symbol and File
Structure, file watcher and reload from disk, the External Tools settings
page, pyrefly as an alternative, inlay hints, semantic
highlighting.

## MVP (milestone "MVP", before the rest)

The first usable, announceable version: M3 first version (#82-#87),
reload from disk (#36), tree file operations (#37), run configurations
(#74), configured interpreters (#75), terminal (#89), debugger (#90), basic
git (#91, #28), settings window (#92, #40), rebrand (#93), release builds
and README / CONTRIBUTING (#94). Terminal, debugger, git and settings get a
recon (`/replica-recon`) after M3's first version. Windows release: per-user
installer without administrator rights, per-machine installer, portable zip.
Also in the MVP: editor soft wraps (#119), the Search Everywhere preview
(#118, one configurable preview component shared with Find in Files),
Python packages and source roots in the project tree (#120), file type
icons by name pattern (#121), navigation history (#103).

Wave 1 (parallel sub-agents in worktrees, merged in this order):

| issue | owner | crates / files | status |
|---|---|---|---|
| #96 Theme import: effect type without a colour | sub-agent small | `themes` | done (PR #111) |
| #101 Flaky db-sqlite lock-wait test on Windows | sub-agent small | `db-sqlite` | done (PR #112) |
| #103 Navigation history (Back / Forward) | sub-agent nav | `workbench` (`navigation.rs`, history hooks), `editor` (jump event), `code-intel-ui` | done (PR #113) |
| #36 Reload files changed on disk | sub-agent sync | new `crates/platform/file-watcher`, `project` (stamps, changes), `text` (reload), `workbench` (file sync, conflict dialog) | done (PR #115) |
| #74 Run configurations | sub-agent run-config | see M2 table | done (PR #117) |
| #37 Project tree file operations | sub-agent tree | `project` (`fs_ops`, trash), `workbench` (project panel), `pyoxide-ide` (Python new items) | done (PR #116) |
| #129 Tree: Reload from Disk in the menu, drag and drop look | sub-agent tree | `workbench` (project panel), `themes` | done (PR #130) |
| #128, #69 Menus: full-row highlight, shortcuts; run dropdown rows (#127) | sub-agent run-config | `ui` (menu) | done (PR #131) |
| #127 Run/Debug Configurations in a window of its own | sub-agent run-config | `ui` (`dialog_window`), `workbench/src/run` | done (PR #132) |
| #134 Tree: full-width root row with hover expansion, indent guides | sub-agent tree | `ui` (`tree_guides`), `workbench` (project panel) | done (PR #137, #141) |
| #135 Run widget inactive icons, toolbar spacing, submenu aim | sub-agent run-config | `ui` (menu), `workbench` (toolbar, run) | done (PR #138, #140) |
| #136 Dialog title bar, option tags, Modify options popup window | sub-agent run-config | `ui` (`dialog_window` popups), `workbench/src/run` | done (PR #139, #140) |

Wave 2 (on command): recon (`/replica-recon`) for terminal (#89),
debugger (#90), git (#91, #28), settings window (#92, #40) and configured
interpreters (#75); small items without a recon: #119, #118, #120, #121.
The settings window reuses `ui::dialog_window` (#127).
Wave 3: rebrand (#93), release (#94), announcement.

Queue: MVP (M3 first version first), then #13 control channel, then #50 (find filters for comments and string
literals, which need the Python syntax layer). Follow-ups of #18 are #24-#42;
of the search work #48-#53 and #65-#69.

Typing latency and the 10 MB file are measured for the reference and Pyoxide (replica/perf.md).
