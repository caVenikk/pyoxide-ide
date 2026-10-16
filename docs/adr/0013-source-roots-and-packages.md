# 0013. Source roots and packages in the project tree

- Status: accepted
- Date: 2026-10-16

## Context
The reference IDE keeps the source folders of a module in its `.iml`
file: `<sourceFolder url="file://$MODULE_DIR$/src" isTestSource="false"/>`
for code, `isTestSource="true"` for tests, a `type` for other kinds
(resources). They are what Python puts on `PYTHONPATH` when a run
configuration adds source roots (`TargetedPythonPaths`: the module's
content roots, then all its source roots, tests included), what names a
module's package (`QualifiedNameFinder`) and what Go to File shows a
file's package from. The project view draws them (IntelliJ Community at
`pycharm/2026.2.3`):

- `SourceRootIconProvider`: a source root gets the icon of its kind
  (`AllIcons.Modules.SourceRoot`, `TestRoot`; in the New UI a blue or a
  green folder), before the excluded folder icon.
- The Python plugin's `PyDirectoryIconProvider` goes first: a module's
  content root gets the module icon (`PythonPyprojectIcons.Model.
  PyProjectModule`, a folder with the Python logo, for a module made from
  `pyproject.toml`, else `AllIcons.Nodes.Module`); an ordinary folder with
  `__init__.py` gets the package icon (`PyUtil.isExplicitPackage`); source
  roots and excluded folders keep theirs.
- `PsiDirectoryNode` draws a content root's name in bold and, in grey, the
  location string of `ProjectViewDirectoryHelper.getLocationString`: the
  lower-case name of its kind of root (`sources root`) and, for a content
  root right below the project, `, ` and a thin space and the folder
  relative to the home folder. Other folders have no hint.
- PyCharm turns on the `pyproject.toml` project model
  (`intellij.python.pyproject.model`, on in PyCharm): every
  `pyproject.toml` makes its folder a module of its own.

*Mark Directory As* (`PyCharm.MarkRootGroup` in `pycharm-core.xml`) has
*Sources Root*, *Excluded*, *Not Excluded*, *Test Sources Root* and
*Unmark*, in this order (`MarkRootActionBase` and its subclasses): a
folder is not offered the kind it has, an excluded folder only *Cancel
Exclusion*, a folder inside a source root only the kinds the module type
allows inside one (`PythonModuleTypeBase.isMarkInnerSupportedFor`: code,
not tests), *Unmark* reads *Unmark as Sources Root* for a root
(`UnmarkRootAction`, `LangBundle`), and the project root can be marked
too. When a folder first opens as a project, PyCharm marks `src` and the
root package folder of `setup.py` as sources roots
(`PythonSourceRootConfigurator`).

Pyoxide found the same roots (`NavigationHooks::source_roots`) for Go to
File and runs, but did not show them, could not change them and ignored
the source folders a project marks in `.idea`. The project tree showed
every folder alike. Platform crates know no language (ADR 0005).

## Options considered
1. Read and write the source folders of the reference's module files:
   rejected for the reasons of ADR 0008 (the format and the files belong
   to the reference).
2. Keep roots out of the project settings and detect them every time:
   marks made in Pyoxide would not survive, and a detected root could not
   be unmarked.
3. Store the roots in `.oxide/project.toml` next to the exclusions, with
   the detected roots and the reference's source folders in effect until
   the project stores its own, and import the reference's once.

## Decision
Option 3.

- Kinds of roots are strings. The platform names the two the reference's
  `.iml` files have, `sources` and `tests` (`project::roots`); a product
  declares the kinds it offers, with their menu label, icon, whether
  they nest inside a source root and whether they come before or after
  *Excluded* (`workbench::RootKind`, `TreeHooks`). Python offers
  *Sources Root* (nests, before *Excluded*) and *Test Sources Root*
  (after it, does not nest).
- `[roots]` in `.oxide/project.toml` lists the folders by kind, `.` for
  the project root, `/`-separated paths relative to the project root; a
  folder has one kind:

  ```toml
  [roots]
  sources = [
      ".",
      "services/api",
  ]
  tests = ["tests"]
  ```

- Once the table is there it holds the project's roots. Before that the
  roots in effect are the reference's source folders (`.idea`, read like
  the exclusions, code and test code roots only) and the roots the
  product detects. Detected roots are rules, not entries: they are stored
  only when the person marks or unmarks a folder or the reference's
  source folders are imported, so opening a project still creates no
  `.oxide` folder.
- The reference's source folders are imported once, together with the
  detected roots, while the settings have no `[roots]` table. The
  exclusions import (ADR 0008) runs first and still only when the project
  has no settings file; a project that has one from before gets its
  roots imported anyway.
- The roots travel with the project's rules (`project::Filter`), so the
  project tree, Go to File and the run requests take them from the
  project tool window. `Filter::source_roots` (kinds `sources` and
  `tests`) are the source roots: `PYTHONPATH` of runs that add source
  roots, the package of a file in Go to File, the language services'
  module names (published for code that has only a path,
  `project::roots::publish`). A change of the roots alone does not build
  the file index again. Roots follow folders the application renames or
  moves, and go with folders it deletes, as the exclusions do.
- A product says which files make a folder a package or a module of its
  own (`project::FolderRules`: Python's `__init__.py` and
  `pyproject.toml`). Listing a folder looks at its subfolders' marker
  files off the UI thread, and a folder looks at its own when it is
  listed again or its marker files change.
- The tree draws, in this order: a module's content root (the project
  root, and a folder that is a module of its own and not excluded) with
  the product's module icon when it has a module marker, else the module
  icon, its name in bold and the hint (the lower-case kind of root, then
  for the project root `, ` with a thin space and its location); a root
  with its kind's icon and no hint; an excluded folder; a package; a
  folder.
- Commands: `project.tree.mark-root {kind, path}` and
  `project.tree.unmark-root {path}` (`.` is the project root), besides
  `project.tree.mark-excluded` and `project.tree.cancel-exclusion`.

## Consequences
- No two-way sync, as for the exclusions: marks made in the reference
  after the import are not picked up.
- The reference's `pyproject.toml` model is followed only as far as the
  tree's looks: a folder with `pyproject.toml` is drawn as a module, but
  it gets no source roots of its own (the model's `src` per module and
  the build backends' package folders), and a module whose project name
  differs from its folder does not show `folder [name]`.
- Not yet: *Namespace Package* (the Python plugin's addition to the
  submenu, with its package icon on the marked folder and the folders
  below it), *Not Excluded* for a folder inside an excluded one,
  *Excluded* for the project root, resource roots, and roots of several
  folders selected at once.
- `NavigationHooks::source_roots` is now the product's detection only;
  the source roots in effect come from the project's rules.
