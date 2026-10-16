# Packages and source roots in the project tree

## Steps
1. Open a folder with `pyproject.toml`, `app/__init__.py`,
   `app/api/__init__.py`, `app/static/site.css`,
   `services/api/pyproject.toml`, `src/lib/core.py`, `tests/test_x.py`
   and no `.idea` or `.oxide` folder. Expand `app` and `services`.
2. Right-click the project root row and choose *Mark Directory As >
   Sources Root*.
3. Right-click `tests`, then `src`, and open *Mark Directory As* each
   time.
4. Mark `services/api` as a sources root.
5. Choose *Mark Directory As > Unmark as Sources Root* on the project
   root and on `src`, then close the project and open it again.
6. Create `services/__init__.py` with *New > File*; delete
   `app/__init__.py` in another program.
7. Rename a folder marked as a test sources root.
8. Open a folder the reference IDE opened before, whose `.idea/x.iml`
   marks the project folder and `services/api` as sources roots
   (`<sourceFolder url="file://$MODULE_DIR$" isTestSource="false" />`)
   and `tests` as a test sources root, and that already has a
   `.oxide/project.toml` with exclusions only.
9. Run `app/api/views.py` with *Add source roots to PYTHONPATH* on, and
   find `core.py` with Go to File.

## Expected
- Step 1: the project root row shows the module icon with the Python
  badge (it has `pyproject.toml`), its name in bold and its location in
  grey. `app` and `app/api` have the package icon, `app/static`,
  `services` and `tests` the folder icon. `services/api` has the module
  icon with the Python badge and its name in bold. `src` is a sources
  root on its own (`src` and the root package folder of `setup.py` are
  detected) with the blue sources root folder and no hint. No `.oxide`
  folder is created.
- Step 2: the submenu of the project root offers *Sources Root* and *Test
  Sources Root* only. Afterwards the root row reads
  `name  sources root, ~/path/to/name` and `.oxide/project.toml` has
  `[roots] sources = [".", "src"]`.
- Step 3: `tests` (inside a sources root) offers *Sources Root* and
  *Excluded*, not *Test Sources Root*. `src` offers *Excluded*, *Test
  Sources Root* and *Unmark as Sources Root*; the rows have the sources
  root, excluded folder and test root icons.
- Step 4: `services/api` keeps its module icon and bold name and shows
  `sources root` after it. A plain folder marked as a root shows the
  root's icon without a hint.
- Step 5: the root row shows only its location again; after reopening
  `src` is an ordinary folder: once the project stores its roots, the
  detected ones no longer apply.
- Step 6: `services` gets the package icon at once, `app` loses it when
  the change on disk is seen.
- Step 7: the folder keeps its test root icon under the new name and the
  settings file follows.
- Step 8: the reference's roots are shown as soon as the project opens,
  together with the detected `src`, and written into `[roots]` of the
  existing settings file; its exclusions stay as they were. Later changes
  in `.idea` are not read.
- Step 9: `PYTHONPATH` holds the project folder and every sources and
  test sources root; Go to File shows `core.py` in `lib`, its package
  from the source root `src`.
- An excluded folder offers only *Cancel Exclusion*; a folder inside it
  offers the kinds of roots but not *Excluded*; a folder a product rule
  excludes (`.venv`) has no *Mark Directory As*.

## Commands
`project.open-folder {"path"}`, `project.tree.reveal {"path"}`,
`project.tree.expand`, `project.tree.context-menu`,
`project.tree.mark-root {"kind", "path"}`,
`project.tree.unmark-root {"path"}`, `project.tree.mark-excluded`,
`project.tree.rename {"path", "name"}`, `project.tree.new-file`,
`project.tree.reload-from-disk {"paths"}`, `workbench.state`
(`tree.rows[].icon`, `tree.rows[].roots` with `root_kind`, `folder`,
`bold`, `hint`; `tree.roots`; `tree.context_menu`).

## Source
IntelliJ Platform Community at `pycharm/2026.2.3` (Apache-2.0):
`PsiDirectoryNode`, `ProjectViewDirectoryHelper.getLocationString`,
`SourceRootIconProvider`, `MarkRootActionBase`, `MarkSourceRootAction`,
`UnmarkRootAction`, `MarkExcludeRootAction`, `ModuleSourceRootEditHandler`,
`JavaModuleSourceRootEditHandler`, `python/resources/META-INF/
pycharm-core.xml` (`PyCharm.MarkRootGroup`), `PyDirectoryIconProvider`,
`PythonModuleTypeBase`, `PythonSourceRootConfigurator`, `PyUtil`
(`isOrdinaryPackage`), `TargetedPythonPaths`, `ProjectBundle` and
`LangBundle`; observed in PyCharm 2026.2.3 (the project root row of a
pyproject.toml project). ADR 0013.
