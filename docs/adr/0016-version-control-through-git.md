# 0016. Version control through the git executable

- Status: accepted
- Date: 2026-10-16

## Context
Basic version control is daily work: file status colours in the project
tree and editor tabs, change markers in the gutter, the Commit tool window
with a diff viewer, rollback, and the branch widget with checkout, new
branch, update and push (#91, #28). The reference runs the `git` program
for every operation and reads a few files of `.git` itself for cheap
repository state (`GitRepositoryReader`). People use the reference and
Pyoxide on the same repositories, with their own git configuration:
hooks, signing, filters and LFS, credential helpers, ssh configuration,
fsmonitor.

Constraints: no git process on the UI thread, ever; Windows, macOS and
Linux; platform crates know no language (ADR 0005); UI crates depend on
traits, not on a backend (AGENTS.md).

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
plugins/git4idea (GitHandler, GitImplBase, GitCommand,
GitRepositoryReader, GitExecutableDetector, GitIndexUtil, status and
commit code), platform/vcs-impl (ChangeListManagerImpl,
UpdateRequestsQueue, FileStatusManagerImpl, LineStatusMarkerDrawUtil),
platform/diff-impl; the reference's version control help pages.

## Options considered
| option | licence | status | writes, auth, behaviour |
|---|---|---|---|
| `git` executable | GPL-2.0 program, run as a process, not linked | the fastest available (untracked cache, fsmonitor, sparse index); a process per call | everything, with the user's configuration: hooks, signing, filters, worktrees, submodules, credential helpers, ssh |
| gix (gitoxide) | MIT OR Apache-2.0 | competitive, parallel | pre-1.0 with API churn; no push; commit without hooks; partial merge and rebase; credentials through helper programs anyway |
| git2 (libgit2) | crate MIT OR Apache-2.0, libgit2 GPL-2.0 with linking exception | slower than git on large trees | C build with OpenSSL and libssh2 on Linux; no hooks; own credential callbacks; diverges from the user's configuration |

Line diff for gutter markers and the diff viewer: `imara-diff` 0.2
(Apache-2.0) or `similar` 3 (Apache-2.0). The `text` crate's reload diff is
cost-limited and not enough.

## Decision
The `git` executable for every operation, Pyoxide's own readers for
repository state, and `imara-diff` for line diffs: it implements git's
histogram algorithm, so markers match the hunks of `git diff`, and it is
built for speed on large files (word-level highlights run it on word
tokens).

### Executable
Lookup: the path from the settings, else `PATH`, then `/usr/local/bin`,
`/opt/homebrew/bin`, `/opt/local/bin`, `/usr/bin`, `/opt/bin`,
`/usr/local/git/bin` (GUI applications on macOS do not get the shell's
`PATH`); on Windows `Program Files\Git\cmd\git.exe`. On macOS `/usr/bin/git`
is used only when the Command Line Tools or Xcode are installed (otherwise
the system shim opens an install dialog); its version check is retried, as
the reference does. Minimum version 2.17 (2.19.2 on Windows), as the
reference; an older or missing git turns version control off with a
notification.

### Process rules
- Every call: `git -c core.quotepath=false -c log.showSignature=false
  <command>`, `GIT_TERMINAL_PROMPT=0` (no terminal); read commands with
  `GIT_OPTIONAL_LOCKS=0`, so a status never rewrites the index and the
  file watcher sees no change of our own making.
- Per repository, write commands (add, checkout, commit, merge, rebase,
  reset, rm, stash, update-index) run one at a time and not during reads;
  reads run in parallel. Stale reads are cancelled.
- Machine-readable output only: porcelain formats with `-z`,
  `for-each-ref --format`. Long path lists are split under the
  command-line limit (`--pathspec-from-file` where the version has it).
- Status: `git status --porcelain=v2 -z --branch --untracked-files=no` for
  tracked changes (renames on) with the branch's ahead and behind, and
  `--porcelain -z --no-renames --untracked-files=all --ignored=matching` for
  untracked and ignored files; both limited to the dirty paths when the
  watcher reported few.
- Base text: `git cat-file --filters HEAD:<path>` (end-of-line and smudge
  filters applied, so no false CRLF changes on Windows), cached by blob.
- Branches: one `git for-each-ref` over `refs/heads`, `refs/remotes`,
  `refs/tags` with upstream and track counts.
- Commit: `git add -- <unversioned files chosen>`, then `git commit --only
  -F <message file> [--amend] [--signoff] [--no-verify] [--author=…] --
  <paths>`: other staged changes stay staged, as in the reference; hooks
  run as git runs them.
- Rollback: `git checkout HEAD -- <paths>` for modified and deleted files,
  `git rm --cached -f` for added ones (and the file deleted when asked).
  Rollback Lines is an undoable edit of the buffer, not a git command.
- Authentication: credential helpers and ssh-agent only; a failure is an
  error notification with git's message. An askpass helper (the Pyoxide
  binary in a helper mode, as the `process` crate's interrupt helper)
  comes later.

### Repository state without a process
Readers for `HEAD`, `refs/**`, `packed-refs`, `config` (remotes, branch
upstreams), the state markers (`MERGE_HEAD`, `rebase-merge/`,
`rebase-apply/`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`) and `gitdir:` files of
worktrees and submodules. A repository using the reftable format
(`extensions.refStorage`) falls back to `git` for refs.

### Refresh
The file watcher's batches (`file-watcher`, `workbench::file_sync`) let the
`.git` paths above, `index`, `ORIG_HEAD` and `info/exclude` through (today
everything under `.git` is dropped) and skip `objects/**`, `logs/**` and
`*.lock`. Changed paths are marked dirty; status runs 300 ms after the last
batch; results are merged into the status map within 100 ms and repainted
(the reference's queues). Events caused by Pyoxide's own commands are
suppressed while they run and followed by one refresh. An edited, unsaved
document of a tracked file is "modified" at once, from its text against
the base.

### Crates
| crate | layer | GPUI | role |
|---|---|---|---|
| `vcs` | platform | no | the model and the backend trait: file status, changes, the status map (ignored ancestors), line ranges of a document against its base (`imara-diff`, recomputed off the UI thread, shifted on edits), branches, commit and rollback requests |
| `git` | platform | no | lookup and version, runner, parsers, readers, the operations above implementing the `vcs` trait, repository detection, `git.*` engine commands; tests against temporary repositories made with `git` |
| `vcs-ui` | platform | yes | Commit tool window, changes tree, diff viewer tab, branch popup and VCS widget, dialogs (Rollback, Push, Update Project, New Branch, checkout problems) |

`vcs-ui` depends on `vcs`, not on `git` (`scripts/check-layers.py`); the
workbench composes them. Repository detection: the repository that
contains the project root; nested repositories and submodules later.

### Storage
The git path and app options in the application settings (ADR 0017); the
commit message draft, recent messages, recent and favourite branches and
the amend author in the personal project state (ADR 0009, `[vcs]`). From
`.idea` only `vcs.xml` is read once: a project mapped to no version
control keeps git off. `.idea` is never written.

## Consequences
- A process per git call: cheap on macOS and Linux, measurably slower on
  Windows, hence batching and dirty-path limits.
- Budgets proposed for `docs/performance.md`, measured in the first issue:
  status of a 10 000-file repository within 150 ms after the watcher batch
  (off the UI thread); gutter markers within 50 ms after the last
  keystroke on a 10 000-line file.
- CI runners have git on all three systems; tests create repositories in
  temporary folders.
- No credential prompts in the first version (documented limitation).
- gix is reconsidered only for read-only hot paths (status, blame, log
  graph) if measurements on a large repository miss the budgets.
- Not in the first version: blame, the log tool window, file history,
  merge conflicts UI, stash and shelve UI, cherry-pick, interactive
  rebase, changelists, partial commits, credential prompts, `.gitignore`
  language support.
