# AGENTS.md

Rules for AI agents and human contributors. Read this file, then
[docs/clean-room.md](docs/clean-room.md), [docs/architecture.md](docs/architecture.md),
[docs/performance.md](docs/performance.md) and [docs/agent-workflow.md](docs/agent-workflow.md).

## Non-negotiable
- **Clean-room.** Never use decompiled, disassembled or otherwise extracted code
  from proprietary products. Never open `.class` files or binaries of PyCharm or
  its plugins. If unsure about a source, stop and ask.
- **No secrets** in code, logs, tests, commits or issues. Passwords come from the
  environment or the OS keychain. `ConnectParams` has a redacting `Debug`; keep it.
- **No personal data** in the repository files: no personal names or emails in
  docs, machine paths, employer or internal hostnames, or links to private
  sessions. The commit author is the maintainer's normal git identity.
- **No conversation history in docs.** Describe the project as it is: what and
  why, never who asked for it, who decided, or what was discussed with an AI
  assistant. Decisions go into ADRs as neutral context, decision and consequences.
- Public text (code, comments, docs, commits, issues) is in English.
- **Keep progress current.** A change that adds or finishes a feature updates its
  row in `replica/features.csv` (`clone`: no / partial / yes) in the same PR, then
  runs `python3 scripts/progress.py`, which regenerates `docs/PROGRESS.md` and the
  README progress block. CI fails if they are out of date.
- No unrelated refactors. Keep changes small and reviewable.

## Language and performance
- All application code is **Rust**. No JVM, Electron or web view. External
  processes are allowed only behind protocols: the Python interpreter, ruff,
  a type checker (ty / basedpyright / pyrefly), debugpy. Anything else needs an ADR.
- Never block the UI thread. I/O and analysis are async, cancellable and
  incremental. Respect the budgets in [docs/performance.md](docs/performance.md);
  regressions fail CI.

## Workflow
- Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo nextest run --workspace`, `cargo test --doc --workspace`. Tests run under
  [cargo-nextest](https://nexte.st) (`cargo install cargo-nextest --locked`; one process per
  test, timeouts and retries for known flaky tests in `.config/nextest.toml`); nextest does not
  run doctests, hence the second command. A new test must pass on its own in a fresh process.
- Layering: `db-core` has no driver dependencies; UI crates depend on traits only.
  Platform crates (`crates/platform/`) know no specific language and never
  depend on product crates (`crates/python/`, ...); see ADR 0005,
  checked by `python3 scripts/check-layers.py`.
- Every user-visible action is a command in the command registry (UI, CLI and
  agent control channel call the same functions).
- Integration tests that need external services are `#[ignore]` and documented.
- Windows, macOS and Linux must keep building; gate platform-specific code with `cfg`.
- Commits: conventional style (`feat(db-core): ...`). Keep a `NOTICE` entry for
  anything copied or closely adapted from open-source code.
- Changes land through pull requests: one branch per issue, CI green on all
  three platforms, squash merge with `Closes #N`. Work-in-progress commits stay
  in the branch.
- The repository holds sources and the assets the application ships (fonts,
  icons) with their licenses. Not committed: screenshots, raw measurement
  output, generated files, throwaway prototypes and their lock files. Results
  go into `replica/perf.md` or an ADR as tables; raw data, if worth keeping, is
  attached to the issue.

## Skills (development method)
`skills/` holds the Replica skill pack (MIT, modified, see [NOTICE](NOTICE)).
Install locally for Claude Code with `mkdir -p .claude && cp -r skills .claude/skills`
(`skills/` is the source of truth; `.claude/` is git-ignored). Read
`skills/PYOXIDE.md` first. Per feature area the loop is
`/replica-recon` -> `/replica-architect` -> `/replica-design` -> `/replica-build`
-> `/replica-backend` -> `/replica-test` -> `/replica-diff` -> `/replica-entrepreneur`;
release work uses `/replica-brand` (sweep), `/replica-launch`, `/replica-deploy`.
Artifacts go to `replica/`. Reference screenshots of proprietary products stay
local in `replica/screens/` (git-ignored) and are never published.

Work with sub-agents uses two more skills in `skills/`: `pyoxide-lead` (the
lead's procedures: briefs, model choice, review, merge, builds to try, usage
pauses, status reports) and `pyoxide-agent` (rules for a sub-agent working on
one issue in its own git worktree, including the report format).

## Private files
`.private/` is git-ignored and for personal notes only. Never reference it from
public files and never commit it.
