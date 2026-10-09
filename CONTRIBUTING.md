# Contributing to Pyoxide IDE

Thanks for helping. This project aims to be fast, small and legally clean, so a
few rules matter more than usual.

## 1. Clean-room rule (non-negotiable)

Do not contribute code, assets or text derived from proprietary software.
That includes decompiled or disassembled code, bytecode, or extracted
resources of PyCharm or any JetBrains plugin. Allowed sources are listed in
[docs/clean-room.md](docs/clean-room.md). Every pull request template asks you
to confirm this. When unsure, ask in the issue first.

Code adapted from open-source projects (for example IntelliJ Community,
Apache-2.0) is welcome with a [NOTICE](NOTICE) entry: source URL, commit, files,
license, changes.

## 2. Setup

```bash
rustup default stable
git clone <repository-url> && cd pyoxide-ide
cargo build --workspace
docker compose up -d      # optional: Oracle Free for database integration tests
```

Windows, macOS and Linux must all keep building. Avoid platform-specific APIs
without `cfg` gates.

## 3. Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Add tests with your change. Tests that need external services are `#[ignore]`
  and documented.
- Performance matters: if you touch a hot path, include measurements and check
  [docs/performance.md](docs/performance.md).
- Never log, print or commit secrets. Passwords come from the environment or the
  OS keychain.
- Keep changes small and focused. Unrelated refactors go in separate PRs.

## 4. Commit and PR style

Conventional commits: `feat(db-core): ...`, `fix(editor): ...`, `docs: ...`.
One logical change per commit. Link the issue in the PR description.

## 5. Issues

Use the issue forms (bug report, feature request). For behaviour that should
match PyCharm, describe the expected behaviour in your own words and link public
documentation; do not paste screenshots or text from proprietary products.

## 6. AI-assisted contributions

Welcome, under the same rules. You are responsible for what you submit: review
it, run the checks, make sure no proprietary material was used. See
[AGENTS.md](AGENTS.md) and [docs/agent-workflow.md](docs/agent-workflow.md).

## 7. Conduct and security

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [SECURITY.md](SECURITY.md).
