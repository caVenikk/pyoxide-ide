# Repository state and statuses

## Steps
1. Open a folder inside a Git work tree (the work tree's root, then a
   folder below it).
2. Outside the IDE: edit a tracked file, create a file, `git add` it,
   `git mv` a tracked file, add `logs/` and `.env` to `.gitignore` and
   create both.
3. Check out a tag (`git checkout v1`), then a branch again.
4. Start a rebase that stops at a conflict.
5. On a repository with `core.autocrlf=true`, open a file checked out
   with CRLF line endings.
6. Open a project whose `.idea/vcs.xml` maps the project folder to no
   version control (`<mapping directory="" vcs="" />`).
7. Start the IDE without any git executable and open a Git project.

## Expected
- Step 1: the repository is found at or above the project folder; for a
  folder below the root only its own files have statuses.
- Step 2: within about half a second after the files are written:
  modified, unversioned, added, renamed (with the old path), and
  `logs/`, everything below it and `.env` ignored.
- Step 3: HEAD is detached at the tag's commit, then on the branch.
- Step 4: the state is rebasing, with the branch being rebased; the
  conflicting file is a conflict.
- Step 5: the file is not changed and its base text has the same line
  endings.
- Step 6: Git stays off for the project.
- Step 7: "Git is not installed" in the status bar; no statuses.

## Commands
`git.status {"path": ...}`, `git.branches`, `git.state`,
`workbench.state` (`vcs`: repository, changes, unversioned, ignored, open
files with their status).

## Source
https://www.jetbrains.com/help/pycharm/enabling-version-control.html,
https://www.jetbrains.com/help/pycharm/file-status-highlights.html,
https://git-scm.com/docs/git-status (porcelain v2),
https://git-scm.com/docs/gitrepository-layout; ADR 0016. The text of the
missing-git notification is to verify against a reference screenshot.
