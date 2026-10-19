# Agent workflow

How AI agents (and humans using them) work in this repository.

## Method
Development runs through the skill pack in `skills/` (see `skills/PYOXIDE.md`).
For each feature area (db, editor, python-intel, run-debug, vcs, db-gui, docker,
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
   `cargo fmt`, `clippy`, `test`, measurements and a short report.
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
- **Third-party notices**: one file per entry in `notices/` (ADR 0019).
- **Counts written by hand** that every feature would change (for example the
  number of theme roles) are left out of the docs; the code is the list.

## Hygiene
Public files are English and contain no personal data. Reference screenshots and
collected user feedback stay local (`replica/screens/`, `replica/reviews.csv`,
both git-ignored). Third-party theme fixtures stay in `tests/fixtures/themes-local/`.
