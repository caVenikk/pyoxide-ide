# A project without Git

## Steps
1. Open a folder that is not in a Git work tree; hover the VCS widget and
   click it.
2. Choose Create Git Repository…; keep the project root; confirm.
3. Create a file in the project tree.
4. Start the IDE with no `git` executable on the PATH and none in the
   usual places; open a Git project.

## Expected
- Step 1: the widget reads "Version Control" without an icon; the
  tooltip "Configure a version control for the project"; the popup offers
  Create Git Repository….
- Step 2: `git init` runs; every file turns unversioned (red); the widget
  shows the default branch; no reopening needed.
- Step 3: the new file is red.
- Step 4: no status colours or widget branch; a notification that Git is
  not installed.

## Commands
`git.init {"path": ...}`, `git.status`, `vcs.state` (widget).

## Source
https://www.jetbrains.com/help/pycharm/enabling-version-control.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`GitToolbarWidgetAction`, `GitBundle`, `VcsBundle`. The popup is to verify
against a reference screenshot.
