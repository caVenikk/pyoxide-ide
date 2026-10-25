---
name: pyoxide-agent
description: Working rules for a sub-agent implementing or researching a Pyoxide IDE issue (Rust/GPUI clone of PyCharm) in its own git worktree - start, sources, clean-room, builds, GUI runs, commits, checkpoint and PR, pause protocol, report format. Use at the start of every Pyoxide sub-agent task, before reading the issue.
---

# Pyoxide sub-agent rules

You work on one GitHub issue of Pyoxide IDE (repo `caVenikk/pyoxide-ide`, a Rust/GPUI clone of PyCharm 2026.2.3). The lead gives you the issue number `N`, a branch name and whether there is a checkpoint. These rules apply unless the lead's prompt says otherwise.

## Start
- You are in your own git worktree (your current directory). First: `git fetch origin && git checkout -b <branch> origin/main` (or the base the lead names).
- Edit files only inside your worktree; never modify the main checkout. Reading the lead's local material (briefs, reference screenshots, the IntelliJ Community checkout) is fine. A scratch folder may be shared with other agents: keep your files in a subfolder named after your issue.
- Read: the issue (`gh issue view N --comments`), `AGENTS.md`, `docs/architecture.md`, the ADRs it touches (`docs/adr/`), `docs/performance.md`, `skills/PYOXIDE.md`, the area in `replica/recon.md`, `tests/scenarios/<area>/`. Read lean: only what the issue needs.

## Reference and clean-room
- Reference behaviour = PyCharm 2026.2.3. IntelliJ Community at tag `pycharm/2026.2.3` (Apache-2.0): `gh api 'repos/JetBrains/intellij-community/contents/<path>?ref=pycharm/2026.2.3' -H 'Accept: application/vnd.github.raw'`, or `git -C <checkout> show pycharm/2026.2.3:<path>` in a local (sparse) clone, e.g. in the git-ignored `.claude/ic`. Read from the tag, not from a working tree that may be on master; never check anything out in a shared checkout.
- Copying or translating Apache-2.0 Community code, strings, metrics and icons is allowed, with a new file `notices/<area>-<name>.md` (source, tag/commit, files, what changed; one file per entry, copy the format of the others; never edit the root `NOTICE`; `python3 scripts/notice.py --check`). Closed-source plugins (Docker, pre-commit, database tools, AI…): no code or assets, draw/write our own from observation.
- Never open PyCharm.app or any JetBrains binary/jar (plugin jars only to extract data files like `plugin.xml` / `*.theme.json`, never `.class`). Never read Zed sources (GPL).
- Derive behaviour from the reference, not from other editors. What the sources cannot settle goes under "needs a reference screenshot" in your report.

## Build and code
- A new worktree has an empty `target/`: run `scripts/seed-target.sh` once before the first build (clones compiled dependencies copy-on-write; see `docs/agent-workflow.md`), and `scripts/check.sh` prints the time of each step.
- Rust: make sure `cargo` is on `PATH` (the lead says if the machine needs a prefix); `export CARGO_BUILD_JOBS=4` when several agents build at once; own `target/` in the worktree.
- Reuse what exists (`ui::dialog_window` incl. non-modal, `ui::form`, `editor::field`, `ui::menu`, `ui::tree_guides`, the tool window registry `workbench/src/tool_windows.rs`, theme roles, navigation history, file sync hooks, `run-config`). Do not build a second one.
- Layering: platform crates know no language (ADR 0005, `python3 scripts/check-layers.py`); Python specifics come through product hooks.
- Hot shared files (`workbench/src/workbench.rs`, `ui/src/menu.rs`, the editor crate): small hunks, logic in your own modules, rebase on `origin/main` before every push, keep everyone's changes.
- Every user-visible action is a registry command with the reference's bindings in the area's file of `workbench/keymaps/macos/` and `default/` (`<area>.toml`; the order of the files is part of the keymap, see `docs/agent-workflow.md`) and a keystroke check in the same area's module `workbench/tests/it/keymap/<area>.rs`.
- Never block the UI thread: file I/O and analysis off the UI thread, cancellable, incremental.
- Windows: items used only under `cfg(unix)` / `cfg(windows)` are gated like their users (no blanket allow); native paths in anything shown.
- New `[workspace.dependencies]` only if the issue/ADR allows it; state the licence in the PR.
- Do not edit `replica/features/`, `README.md`, `replica/plan.md` (the lead does). Never commit generated progress output: the report is published from `main` (ADR 0020).

## GUI runs and visual parity
- Only `PYOXIDE_BACKGROUND=1` runs (non-activating, behind other windows) with `PYOXIDE_SHOT=<png>` / `PYOXIDE_RUN=<json commands>`. Never a foreground window, never `osascript`/System Events. No `PYOXIDE_BENCH` runs unless the lead says the machine is free (bench windows stay on top of everything). Close every process you start.
- Visual parity is part of done: release build, screenshots of every new window/popup state, compared with the reference screenshots (kept locally, never committed, see `docs/clean-room.md`) or the sources; list remaining differences.
- Check the real app path once (release build, real entry points), not only headless tests.

## Commits, checkpoint, PR
- Conventional commits (`feat(area): ...`), `cargo fmt --all` before each. NEVER a `Co-Authored-By` trailer.
- Before a push: `scripts/check.sh` (what CI runs: fmt, clippy for the host and for Windows, tests, doctests, progress, layer and notice checks; `--quick` for the changed crates and their dependents while iterating, the full run before the last push). Tests run under cargo-nextest (one process per test; `.config/nextest.toml`): a test must pass in a fresh process on its own, and a flaky test is fixed, not added to the retry list.
- Checkpoint (if the lead asked for one): after stage 1 commit, do NOT push, report and end; the lead resumes you.
- Otherwise: push only your branch, open ONE ready PR (`Closes #N`, body ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`). Never merge.
- No secrets or personal data in code, tests, commits, PR text; screenshots of real projects stay local.

## Pause protocol
- On "стоп": commit WIP as `wip: paused, not checked` (no checks, no push), report in <= 5 lines, end. On "продолжай": rebase on origin/main if it moved, continue.

## Report format
- FIRST line: `STATUS: DONE`, `STATUS: PARTIAL` or `STATUS: BLOCKED` + one sentence.
- Claims with evidence: the command and the decisive output line, commit SHAs, screenshot paths. "Tests pass" without the command is not evidence.
- Then: commits; what works; tests; deviations from the reference; remaining visual differences; what needs a reference screenshot; follow-ups; PR URL. Checkpoint <= 40 lines, final <= 60 lines. Details go into the PR body.
