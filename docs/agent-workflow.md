# Agent workflow

How AI agents (and humans using them) work in this repository.

## Method
Development runs through the skill pack in `skills/` (see `skills/PYOXIDE.md`).
For each feature area (db, editor, python-intel, run-debug, vcs, db-gui, sql, docker,
settings-import, packaging):

| Step | Skill | Output |
|---|---|---|
| 1 | `/replica-recon` | behaviour map, feature matrix in `replica/features/`, scenarios in `tests/scenarios/<area>/` |
| 2 | `/replica-architect` | ADRs in `docs/adr/`, module boundaries |
| 3 | `/replica-design` | themes and tokens, contrast checks |
| 4 | `/replica-build` | panels, dialogs, commands, vertical slices |
| 5 | `/replica-backend` | engine services (LSP/DAP, DB drivers, git, Docker, keychain) |
| 6 | `/replica-test` | unit, integration, scenario, snapshot and performance tests |
| 7 | `/replica-diff` | parity score, layout and behaviour diffs, performance deltas |
| 8 | `/replica-entrepreneur` | public user feedback -> improvements and priorities |

Release: `/replica-brand` (sweep for trademarked names), `/replica-launch`,
`/replica-deploy`.

## Parallel work
1. A lead agent keeps `replica/plan.md` (areas, dependencies, owners, status)
   and commits contracts (traits, command schemas, formats) first.
2. Then 3-4 sub-agents run in parallel, each in its own git worktree and branch,
   on independent crates or areas. One owner per file set.
3. The editor core and UI toolkit are built sequentially by a single agent until
   interfaces stabilise.
4. Each sub-agent runs the skill loop for its area and finishes with
   `scripts/check.sh` (see Checks), measurements and a short report.
5. The lead reviews the sub-agent's branch, opens a pull request that
   references the issue, waits for CI on Windows, macOS and Linux, squash-merges
   it and updates the plan, the feature files and the roadmap.
6. Prototypes built to compare options stay out of `main`: their conclusions and
   numbers go into the ADR, raw data into the issue.

## Files that parallel pull requests share
Pull requests are developed in parallel and merged in any order, and each
rebase costs a CI run. So a change adds its own small files instead of
appending to a shared one, and nothing generated is committed:
- **Feature status**: one CSV file per feature,
  `replica/features/<area>/<name>.csv` (ADR 0020). A pull request edits the
  `clone` column of its features or adds a file for a new one;
  `python3 scripts/progress.py --check-data` validates the files (the
  `progress` CI job). The report is generated, not committed: the `progress`
  job shows it in its summary, and the Progress workflow publishes it to
  GitHub Pages after each change on `main` (the README links the page and its
  badge). `python3 scripts/progress.py` prints it locally.
- **Keymaps and their checks**: the bindings of the workbench are one file per
  area, `crates/platform/workbench/keymaps/macos/<area>.toml` and
  `keymaps/default/<area>.toml` (`editor`, `find`, `vcs-commit`, ...), and the
  action table one file per menu, `keymaps/actions/<menu>.toml`. A feature
  adds its bindings to its area's file in both keymaps and its keystroke
  checks to `tests/it/keymap/<area>.rs`, whose two tests
  (`keymap::<area>::every_macos_binding_works` and
  `..._every_windows_and_linux_binding_works`) start their own app with the
  complete keymap and press only that file's bindings, in parallel with the
  other areas. A new area is a new file in both folders, one line in the
  `concat!` lists of `src/keymap.rs` (and `src/keymap/actions.rs`) and a
  module in `tests/it/keymap/mod.rs`; a test fails for a file that is not
  joined once or has no module. The order of the files in the lists is part
  of the keymap (of two bindings of one keystroke in the same innermost
  context, or one of them without a context, the later wins, so the file of
  a popup's Escape comes after the editor's): put a new file after the files
  whose bindings it must win over, and do not reorder the lists.
- **Third-party notices**: one file per entry in `notices/` (ADR 0019).
- **Counts written by hand** that every feature would change (for example the
  number of theme roles) are left out of the docs; the code is the list.

## Checks
`scripts/check.sh` is the one definition of the checks: CI's `check` job
calls it step by step, so a local run uses the same commands and flags as
CI. On Windows, `scripts/check.ps1` runs it with the bash of Git for Windows.

| command | when | what |
|---|---|---|
| `scripts/check.sh --quick` | while iterating | the crates changed against `origin/main` (committed or not) and the crates that depend on them |
| `scripts/check.sh` | before the last push | every step on the whole workspace |
| `scripts/check.sh clippy test` | any time | only the named steps (`--help` lists them) |

The steps: `fmt`, `clippy`, `clippy-windows` (clippy for
`x86_64-pc-windows-gnu`, which catches items used only under `cfg(unix)`
before the Windows job does; needs `rustup target add x86_64-pc-windows-gnu`
and mingw-w64, skipped with a note without them), `build` and `test`
(cargo-nextest, profile `ci`), `throughput` (terminal, ADR 0014), `doc`
(doctests) and `repo` (progress, layer and notice checks).

What CI runs, and when (`.github/workflows/ci.yml`):
- One run per pull request: a new push cancels the run of the push before.
  Runs on `main` are never cancelled.
- The OS matrix (`check` on Linux, macOS and Windows) and `integration` run
  on `main` and for pull requests that are ready for review and change more
  than documentation (`*.md`, `docs/`, `replica/`, `tests/scenarios/`). For a
  draft or a documentation-only pull request their jobs pass without running
  anything, so the required checks report; marking a draft ready for review
  starts the full run. Push checkpoints as drafts.
- Pull requests restore the build cache of `main` and do not save their own.
- The checks build with the workspace's dev profile, as a local debug build
  does (optimized dependencies, ADR 0007), so `scripts/check.sh` reuses what
  `cargo test` compiled. CI adds only settings that change no result: line
  tables instead of full debug info, no incremental compilation (measured in
  `replica/perf.md`, CI).

## Test repositories
A test that needs a Git repository makes it with `git::testing::init` (or
`git::testing::clone`), never with a bare `git init` / `git clone`: they pin
`core.autocrlf=false` and `core.eol=lf` before the first file is written,
because Git for Windows defaults to `autocrlf=true` and converts line endings
there only.

## Hygiene
Public files are English and contain no personal data. Reference screenshots and
collected user feedback stay local (`replica/screens/`, `replica/reviews.csv`,
both git-ignored). Third-party theme fixtures stay in `tests/fixtures/themes-local/`.
