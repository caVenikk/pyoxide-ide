# Agent workflow

How AI agents (and humans using them) work in this repository.

## Method
Development runs through the skill pack in `skills/` (see `skills/PYOXIDE.md`).
For each feature area (db, editor, python-intel, run-debug, vcs, db-gui, docker,
settings-import, packaging):

| Step | Skill | Output |
|---|---|---|
| 1 | `/replica-recon` | behaviour map, `replica/features.csv`, scenarios in `tests/scenarios/<area>/` |
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
   it and updates the plan, `features.csv` and the roadmap.
6. Prototypes built to compare options stay out of `main`: their conclusions and
   numbers go into the ADR, raw data into the issue.

## Hygiene
Public files are English and contain no personal data. Reference screenshots and
collected user feedback stay local (`replica/screens/`, `replica/reviews.csv`,
both git-ignored). Third-party theme fixtures stay in `tests/fixtures/themes-local/`.
