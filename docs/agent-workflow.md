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
   `scripts/check.sh --quick --base origin/<target-branch>`, the focused
   regression/scenario, measurements where relevant, and a short report.
5. The lead reviews task PRs into a protected `integration/*` branch. After
   2-4 tasks, a batch PR into `main` runs the complete three-platform and
   database checks. Merge through GitHub squash after the required gate passes;
   close the included issues in that batch PR. Direct PRs to `main` use the
   same full gate. Keep working on independent tasks while CI runs.
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
  whose bindings it must win over, and do not reorder the lists. A module
  (ADR 0022) keeps its own folders in its crate
  (`keymaps/{macos,default}/<area>.toml`), joins them the same way into its
  `workbench::KeymapFiles` (joined after the workbench's files) and checks
  them in its own tests with `workbench::test_support::keymap` (feature
  `test-support`): `check_area` per area and keymap, `check_joined_once` per
  folder.
- **Third-party notices**: one file per entry in `notices/` (ADR 0019).
- **Counts written by hand** that every feature would change (for example the
  number of theme roles) are left out of the docs; the code is the list.

## Checks

`scripts/check.sh` defines the commands used locally and in CI; the PowerShell
wrapper passes the same arguments to Bash on Windows.

| Command | Purpose |
|---|---|
| `scripts/check.sh --quick --base origin/<target-branch>` | Check changed crates and all reverse dependencies before a task push |
| `scripts/check.sh --quick --base origin/<target-branch> clippy test` | Focused iteration on selected stages |
| `scripts/check.sh` | Explicit whole-workspace validation when required |

The default local base remains `origin/main` for compatibility. Always pass
an explicit base for a batch branch. Scope includes normal, development and
build dependencies, both sides of renames and removals. Build configuration,
lockfile, shared fixtures and unknown changes require conservative full scope.
Formatting and repository consistency checks remain cheap and unconditional.
A bug fix also exercises its regression; UI changes exercise the real app
path. A small code change does not require a fresh unrelated benchmark.

CI modes (ADR 0024):
- **Task:** ready PR into `integration/*`; affected crates, appropriate
  platforms, and database integration only when relevant. Shared native
  components or uncertain platform impact expand the checks conservatively.
  Task PRs in a batch run in parallel against one frozen integration base.
  Wait for every included task's required checks and review, then squash their
  checked heads into integration. Do not rebase and repeat CI merely because a
  preceding task from the same verified batch merged. Conflicts or source
  changes require relevant checks again. Integration requires checked heads;
  only main requires the branch to be up to date before merging.
- **Full:** ready PR into `main`, an explicit dispatch, or a scope fallback;
  workspace clippy, tests and doctests on Linux/macOS/Windows, database
  integration and the existing terminal throughput check.
- **Draft:** inexpensive checks only; live API checks prevent a stale draft
  skip from passing after the PR becomes ready.
- **Verified main:** a push may reuse a successful full PR result only when
  the merged PR, tested source tree, configuration and toolchain evidence
  match. A matching full-main cache seed from the preceding six days is also
  required, so dependency/configuration changes refresh the shared main caches.
  This is seed eligibility, not a guarantee that GitHub retained every cache.
  Missing, stale or invalid evidence runs full CI instead. Artifact
  builds and smoke checks remain on main.

`CI required` is the single required aggregate check. It validates the plan,
actual jobs and current PR base/head/draft state. Failure, cancellation,
missing jobs or an unexpected skip cannot count as a completed full check.
Rulesets require the branch to be up to date, preserve squash-only PR merges,
prevent deletion/force pushes, and provide no bypass actors. The batch branch
has its own required gate; a fast result cannot satisfy a main batch's gate.

Task branches start from the named batch branch. Rebase against the actual PR
target, not unconditionally against main. Keep each batch small and create
its next branch from the latest main. Publish one try build after the batch,
while keeping individual regression evidence. If a final combined check fails,
fix it in the batch before merging to main. Do not postpone all integration
validation until release.

Each worktree keeps its own target directory. Seed it before the first build
with `scripts/seed-target.sh` (or `--from <other-target-directory>`); the helper
copies compiled dependencies with copy-on-write where available and respects
Cargo's build locks. Do not concurrently build into one shared target.
`scripts/check.sh` reports per-stage timing. GitHub cache restore is not proof
of workspace artifact reuse: record the restored key, cache hit and compilation
work when assessing an optimization. Preserve the assertions and avoid blanket
retries or timeout inflation.

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
