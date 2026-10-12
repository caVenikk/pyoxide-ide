# Excluded folders and ignored files

## Steps
1. Open a folder that contains `src/app.py`, `docs/index.md`, `build/out.txt`,
   a `.git` folder, a `.mypy_cache` folder, `src/app.pyc`, a `.venv` folder
   and a `.gitignore` that names `build/`.
2. Look at the project tool window.
3. Right-click `docs` and choose *Mark Directory As > Excluded*.
4. Expand `docs` with its chevron.
5. Right-click `docs` again and choose *Mark Directory As > Cancel
   Exclusion*.
6. Mark `docs` as excluded again, close the project and open it again.
7. Open a folder that the reference IDE opened before, whose `.idea/x.iml`
   excludes `venv` (`<excludeFolder url="file://$MODULE_DIR$/venv" />`),
   and that has no `.oxide` folder yet. Then mark `venv` as not excluded
   in the reference IDE and open the folder in Pyoxide again.
8. Open a folder that has neither `.idea` nor `.oxide` and no folder to
   exclude.

## Expected
- Step 2: `.git`, `.mypy_cache` and `src/app.pyc` are not shown (the
  default list of ignored files and folders). `.venv` is shown as an
  excluded folder: the excluded folder icon, the row background of
  non-project files, the name in the normal text colour, collapsed.
  `build` is an ordinary folder: files that version control ignores are
  ordinary content. `.gitignore` itself is shown.
- Step 3: the menu has *Mark Directory As* with *Excluded*; after choosing
  it `docs` looks like `.venv` and stays collapsed. Expand All leaves it
  closed. A `.oxide/project.toml` file appears with `docs` under
  `[exclusions] paths`.
- Step 4: an excluded folder expands by hand; its subfolders show the
  excluded folder icon and all its rows the non-project background.
- Step 5: the submenu offers *Cancel Exclusion* instead of *Excluded*;
  afterwards `docs` is an ordinary folder again and the settings file no
  longer lists it.
- Step 6: `docs` is still excluded.
- Step 7: `venv` is excluded as soon as the project opens and a
  `.oxide/project.toml` lists it; `.idea` is unchanged. After the change in
  the reference IDE `venv` stays excluded in Pyoxide: `.idea` is read only
  once, when there is no `.oxide` yet.
- Step 8: no `.oxide` folder is created.
- Find in Files, Go to File and indexing leave hidden and excluded entries
  out.

## Commands
`project.open-folder {"path": ...}`, `project.tree.reveal {"path": ...}`,
`project.tree.expand`, `project.tree.expand-all`, `workbench.state`
(`tree.rows[].excluded`).

## Source
https://www.jetbrains.com/help/pycharm/configuring-project-structure.html,
https://www.jetbrains.com/help/pycharm/content-root.html,
https://www.jetbrains.com/help/pycharm/settings-file-types.html,
ADR 0008.
