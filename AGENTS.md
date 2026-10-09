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
- **No personal data** about maintainers or users in the repository: no names,
  emails, machine paths, internal hostnames, or links to private sessions.
  Committer identity in this repository is the neutral project identity.
- Public text (code, comments, docs, commits, issues) is in English.
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
  `cargo test --workspace`.
- Layering: `db-core` has no driver dependencies; UI crates depend on traits only.
- Every user-visible action is a command in the command registry (UI, CLI and
  agent control channel call the same functions).
- Integration tests that need external services are `#[ignore]` and documented.
- Windows, macOS and Linux must keep building; gate platform-specific code with `cfg`.
- Commits: conventional style (`feat(db-core): ...`). Keep a `NOTICE` entry for
  anything copied or closely adapted from open-source code.

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

## Private files
`.private/` is git-ignored and for personal notes only. Never reference it from
public files and never commit it.
