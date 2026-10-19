---
name: pyoxide-lead
description: The lead's procedures for Pyoxide IDE development with sub-agents - running issues (brief, model choice, checkpoints), reviewing and merging PRs, release builds for the maintainer to try, usage-limit pauses, status reports. Use at the start of every Pyoxide lead session and whenever spawning, resuming, reviewing or merging agent work.
---

# Pyoxide lead procedures

The lead is the session that plans, delegates and reviews; sub-agents implement in their own git worktrees and follow the `pyoxide-agent` skill. Project rules: `AGENTS.md`, `docs/clean-room.md`, `docs/architecture.md`, `skills/PYOXIDE.md`. Machine-specific facts (paths, test projects, where builds for the maintainer go) and the maintainer's standing decisions are kept in the lead's local notes, not here.

## Spawning an agent
- One issue = one background agent in its own git worktree; independent issues start in parallel (one message, several agent calls). Prefer plain agents over a scripted workflow unless the work is a large fan-out.
- Prompt = short: issue number, branch `feat/<n>-<slug>`, base, checkpoint yes/no, owned files and who else works in parallel (hot files), local reference screenshots to compare with, local paths the agent needs, and: "Follow the `pyoxide-agent` skill." Do not repeat the skill's rules in the prompt.
- Model per task: the strongest model for implementation in Rust/GPUI, architecture and parity work; a mid-size model for research reading, recon docs, scenarios and issue drafts; the smallest model for mechanical work (bookkeeping, code search, small edits, CI log triage).
- Follow-ups go to a fresh agent with a short brief (or a small fix by the lead). Resuming an agent re-reads its whole context: resume only for an agreed checkpoint or continuation.
- Keep a local list of running agents with their issues, so a pause or a context compaction does not lose them.

## Reviewing a checkpoint or PR
1. Read the report (the `STATUS:` line, evidence). Look at the diff where it matters: hot files, file I/O or blocking calls on the UI thread, `unwrap` outside tests, layering (`python3 scripts/check-layers.py`).
2. Visual check: the agent's release build, `PYOXIDE_BACKGROUND=1 PYOXIDE_SHOT=<png> [PYOXIDE_RUN=<json>]` on a small sample project of the area, compared with the reference screenshots. Missing references: ask the maintainer once, as one list with a file name and exact steps per shot. macOS has no `timeout`: `perl -e 'alarm 60; exec @ARGV' <cmd>`.
3. Settle reference details in the sources (tag `pycharm/2026.2.3`) yourself; ask the maintainer only what the sources cannot settle (pure rendering details, closed-source plugins).
4. Differences: small ones -> a follow-up issue with exact items; real bugs -> back to the agent as a numbered must-fix list.

## Merging
- Bookkeeping in the PR branch: the feature's file in `replica/features/<area>/` (`clone` yes / partial; a new feature is a new file `<name>.csv`, header and one row) + `python3 scripts/progress.py --check-data`, commit `docs(progress): ... (#N)`. Nothing generated is committed: the Progress workflow publishes the report to GitHub Pages after the merge (ADR 0020).
- Third-party notices: a PR that copies or adapts open-source code adds its own file `notices/<area>-<name>.md` (one per entry, checked by `python3 scripts/notice.py --check` in CI); the root `NOTICE` is not edited. A branch that still edits the root `NOTICE` moves its entry into such a file when it rebases.
- When main moved: rebase (or merge main into a branch others build on); after conflicts run clippy and `cargo nextest run --workspace` (plus `cargo test --doc --workspace`) before pushing. Check the exit code of `git rebase` itself; never let a script continue past a failed rebase.
- Merge through auto-merge after review: `gh pr ready <n>` (if draft), `gh pr merge <n> --auto --squash --subject "<title> (#n)" --body "... Closes #N"`. Do not poll CI; disable auto-merge (`--disable-auto`) before pushing further changes. Squash merge, never a `Co-Authored-By` line. Pull requests from forks are merged only with a maintainer's explicit OK.
- Clean up after the merge: once the PR is merged and the maintainer has tried the build (see "Builds for the maintainer"), remove the agent's worktree and its branch (`git worktree remove --force <path>`, `git branch -D <branch>`; a squash-merged branch is not seen as merged). Each worktree holds a 10-27 GB `target/`; keep only work in progress.

## Builds for the maintainer
- After user-visible merges: update main, `cargo build --release -p pyoxide-ide`, a quick screenshot, then replace the maintainer's copy atomically (`cp` to `<name>.new`, then `mv -f`), so a running copy is not corrupted.
- Give the launch command and 2-4 concrete things to try per issue, with the comparison to PyCharm. Keep agent worktrees until the maintainer has tried the build.

## Usage limits
- Check the plan usage on every agent notification and with background timers: rarely when far from the maintainer's threshold, every 2-3 minutes near it. The maintainer sets the threshold per window; start the stop early enough for the agents' WIP commits.
- Stop = "стоп" to every running agent (WIP commit, <= 5-line report), a fallback timer, stop stragglers, check for stray processes (`pgrep -fl 'target/release/pyoxide-ide|cargo'`).

## Status messages (in the maintainer's language, short)
- **In progress**: `#N — what — who`, one line each, links on the numbers.
- **Ready to check**: per issue the launch command in a code block + 2-4 things to try.
- **Waiting for you**: decisions, OKs, screenshots, one line each.
- A usage line at the end. No long narratives; details only when asked.
