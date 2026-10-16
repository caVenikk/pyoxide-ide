# VCS widget and branches

## Steps
1. Open a Git project on `main` with a remote; hover the VCS widget;
   click it.
2. Choose New Branch…, type `feat/x`, press Create.
3. Open the popup; type `ma`; open the submenu of `main`; choose
   Checkout.
4. Modify a file that differs between the branches and check out
   `feat/x`.
5. Check out a tag through Checkout Tag or Revision….
6. Check out `main`; rename `feat/x` (F2) to `feat/y`; delete it.
7. Try New Branch… with an empty name, an existing name and `HEAD`.

## Expected
- Step 1: the widget shows the branch icon and `main`; the tooltip "Git
  Branch: main"; the popup has the search field "Search for branches and
  actions", Update Project…, Commit…, Push…, New Branch…, Checkout Tag or
  Revision…, groups Recent, Local and Remote; `main` is the current
  branch and a favourite (star).
- Step 2: the dialog "Create New Branch" with "Branch Name:" and
  "Checkout branch" ticked; afterwards the widget shows `feat/x` and a
  balloon "Checked out new branch feat/x from main"; in the popup
  `feat/x` sits in a `feat` folder.
- Step 3: the search filters branches and actions; the submenu offers
  Checkout, New Branch from 'main'…, Compare with 'main', Rebase, Merge,
  Update, Push…, Rename…, Delete.
- Step 4: "Git Checkout Problem" with Smart Checkout, Force Checkout and
  Don't Checkout; Smart Checkout stashes, checks out and restores the
  changes.
- Step 5: the widget shows a warning icon and the tag name (or an 8
  character hash for a revision); the tooltip says HEAD is detached.
- Step 6: the widget follows; "Deleted branch feat/y" with Restore.
- Step 7: "Specify name for the new branch", "Branch name … already
  exists", "HEAD is a reserved keyword".

## Commands
`git.branches`, `git.new-branch {"name": ...}`, `git.checkout
{"ref": ...}`, `git.rename-branch`, `git.delete-branch`, `vcs.state`
(widget text, popup groups).

## Source
https://www.jetbrains.com/help/pycharm/manage-branches.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `GitToolbarWidgetAction`,
`GitBranchesTreePopup`, `GitBundle`, `DvcsBundle`. The items per branch
kind are to verify against a reference screenshot.
