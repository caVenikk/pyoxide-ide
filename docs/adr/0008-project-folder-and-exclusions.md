# 0008. Project folder and exclusions

- Status: accepted
- Date: 2026-10-12

## Context
The reference IDE sorts the files of a project into three kinds, and Find
in Files (#43), Go to File (#44) and indexing depend on the same split:

- **Ignored files and folders** (the "Ignored Files and Folders" list under
  File Types): name masks that are never shown, indexed or searched. The
  default list is `FileTypeManagerImpl.DEFAULT_IGNORED`: `*.pyc`, `*.pyo`,
  `*.rbc`, `*.yarb`, `*~`, `.DS_Store`, `.git`, `.hg`, `.mypy_cache`,
  `.pytest_cache`, `.ruff_cache`, `.svn`, `CVS`, `__pycache__`, `_svn`,
  `vssver.scc`, `vssver2.scc`. A mask is an exact name (case-sensitive),
  `*.ext` (the extension, any case) or a `*`/`?` wildcard
  (`FileNameMatcherFactoryImpl`, `FileTypeAssocTable`,
  `WildcardFileNameMatcher`).
- **Excluded folders**: stored per module as `excludeFolder` (and
  `excludePattern`, name masks matched below the content root) in the
  module's `.iml` file. They stay in the Project tool window with the
  excluded folder icon (`SourceRootIconProvider`: any excluded folder, also
  below an excluded root) and the "Non-Project Files" file colour as the
  row background (`NonProjectFilesScope`, colour `FileColor.Yellow`); the
  name keeps its normal colour. They are not expanded automatically but can
  be expanded by hand, and they are left out of project search, Go to File
  and indexing. A directory search skips excluded subfolders unless the
  searched folder itself is excluded (`FindInProjectTask`). They are set
  with *Mark Directory As > Excluded* and removed with *Cancel Exclusion*
  in the Project tool window's context menu (`MarkExcludeRootAction`,
  `UnmarkRootAction`, the `PyCharm.MarkRootGroup` action group).
- **Everything else**, including files that version control ignores; those
  are only coloured by version control.

The Python plugin excludes virtual environments on its own and writes them
into the module file: every folder named `.venv` in the project, at
project open and whenever files are created (`VenvExcluder`,
`VirtualEnvReader.DEFAULT_VIRTUALENV_DIRNAME`), and the environment of the
interpreter when one inside the project is chosen
(`excludeInnerVirtualEnv`). A folder is not excluded just because it
contains `pyvenv.cfg`.

`.idea/modules.xml` (`ProjectModuleManager`) lists the module files by
`filepath="$PROJECT_DIR$/..."`; a module file's `NewModuleRootManager`
holds `content` entries with their exclusions. `$PROJECT_DIR$` is the
folder that contains `.idea`; `$MODULE_DIR$` is the folder of the module
file, or the project folder when the module file is inside `.idea`
(`JpsProjectLoader`, `JpsModuleRootModelSerializer`, `PathMacroUtil`).
The `.idea` folder itself is shown in the Project tool window while
excluded files are shown (the default), and Find in Files leaves the files
in it (and `.iml`, `.ipr`, `.iws` files) out unless asked to include them
(`ProjectCoreUtil.isProjectOrWorkspaceFile`, `FileScanner`).

People use the reference IDE and Pyoxide on the same projects and mark
virtual environments and build folders as excluded in the reference.
Those exclusions should hold in Pyoxide as soon as it opens the project.
Pyoxide also needs a place of its own for project settings, starting with
exclusions.

Sources: IntelliJ Platform Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @
0b33b8b8e90353a31a9db5b8fcecc6626d94967e; files listed in NOTICE.

## Options considered
1. Read and write the reference's module files. Changes would show up in
   both IDEs, but the format belongs to the reference, which rewrites the
   files itself, and both IDEs editing them at once could lose changes.
2. Per-project state in the application data folder, keyed by the project
   path. Invisible to the team, lost when the project moves.
3. A folder of Pyoxide's own in the project root with a settings file,
   importing the reference's exclusions once.

## Decision
Option 3.

### Project folder
- Pyoxide keeps project settings in `.oxide/` in the project root. The
  name is provisional and lives in one constant, `project::STORE_DIR`.
- Shared settings are in `.oxide/project.toml`, which is meant to be
  committed: exclusions are shared project settings.
- The format is TOML with a `version` (now 1) and an `[exclusions]` table:
  `paths`, relative to the project root with `/` separators on every OS,
  and `patterns`, name masks matched below the root.

  ```toml
  version = 1

  [exclusions]
  paths = [
      ".venv",
      "docs/build",
  ]
  patterns = ["*.egg-info"]
  ```

- Keys and comments Pyoxide does not know survive a rewrite (`toml_edit`),
  so later settings can be added without losing data. A file with a
  higher version is read as far as it is understood and never rewritten.
- Writes replace the file at once: a temporary file in `.oxide/`, flushed,
  then renamed over the old one.
- `.oxide/` is created only when there is something to store: an import
  found exclusions, or the person marked a folder. Opening a project never
  creates an empty folder.

### Import from the reference
- When a project is opened and has no `.oxide/` yet, Pyoxide reads the
  exclusions of `.idea` (the module files listed in `modules.xml`, or the
  `.iml` files in `.idea` without it; `excludeFolder` entries inside the
  project, and the `excludePattern` entries of a content root at the
  project root) and saves them into `.oxide/project.toml`.
- After that `.oxide/` is the only source. Pyoxide never writes into
  `.idea` and never reads it again for exclusions.

### Hidden and excluded
- Hidden (never shown, never searched): the reference's default ignored
  list above, and `.oxide/`. The reference shows `.idea` and only leaves
  it out of Find in Files; Pyoxide hides its own folder from both, and
  shows `.idea` as an ordinary folder.
- Excluded: the paths of the project settings and everything below them,
  names matching the patterns, and the folders a product excludes on its
  own. The rules are one type, `project::Filter`, which the project tree,
  Find in Files, Go to File and indexing all use.
- Files matched by `.gitignore` are ordinary content.
- Platform crates know no language, so a product registers its automatic
  exclusions (`project::AutoExclusions`, set from `workbench::Product`).
  The Python product excludes every folder named `.venv`, as the
  reference does, and a folder directly under the project root that
  contains `pyvenv.cfg`, standing in for the reference's exclusion of the
  interpreter's environment. Once interpreters are chosen in the IDE,
  that rule becomes "the selected interpreter's environment".
  Automatic exclusions are rules, not stored entries, so they create no
  `.oxide/` folder, and they cannot be cancelled: the reference excludes
  them again on its own.
- `Filter::is_project_config` reports the reference's settings files
  (anything in `.idea`, `*.iml`, `*.ipr`, `*.iws`). They are shown and not
  excluded; Find in Files holds their matches back behind a link that
  includes them, as the reference does.
- The rules are loaded off the UI thread when a project opens; the tree
  lists folders once they are there. The project tool window owns the
  current rules (`ProjectPanel::filter`, a cheap clone) and replaces them
  when the person marks or unmarks a folder, so Find in Files, Go to File
  and indexing take them from there rather than building their own.

### Marking folders
- The project tool window's context menu (a reusable component in `ui`,
  `ui::menu::ContextMenu`) has *Mark Directory As* (*Mark File As* for a
  file) with *Excluded* for project content and *Cancel Exclusion* for a
  stored exclusion, the labels of the reference. Both are UI commands,
  `project.tree.mark-excluded` and `project.tree.cancel-exclusion`, with an
  optional `path`; the menu is `project.tree.context-menu` (the context
  menu key on Windows and Linux).
- The menu follows the reference's popup menus in the New UI: item rows of
  the list row height plus 1 px above and below (26 px with the Islands
  themes), a rounded selection 7 px from the sides with 6 px inside,
  separators of 9 px, 6 px above the first and below the last row, 8 px
  corners, a 1 px border in the theme role `popup-border` (the reference's
  `Popup.borderColor`) and the 16 px `Ide.Shadow` in both themes.
- Excluded rows keep the normal text colour and get the excluded folder
  icon (folders) and the background of non-project files across the row
  (the theme role `non-project-file`, the reference's `FileColor.Yellow`).
  They are not expanded on their own (Expand All skips them) and expand by
  hand.
- The rows and the rules change at once; the settings file is written in
  the background, one save at a time.

## Consequences
- There is no two-way sync. Exclusions made in the reference after the
  import are not picked up, and exclusions made in Pyoxide do not reach
  the reference. A command that imports `.idea` again can follow.
- Projects get a committable `.oxide/` folder once they have settings.
- Exclude patterns of content roots below the project root and modules
  that the reference has unloaded are not taken into account by the
  import.
- The other rows of the reference's *Mark Directory As* (Sources Root,
  Test Sources Root, Namespace Package, Not Excluded for a folder inside an
  excluded one) and the user's own list of ignored files and folders come
  later, as does the rest of the context menu (file operations, #37).
  Sources Root and Test Sources Root: ADR 0013.
- Later, with the settings task: personal state such as open tabs and the
  window layout, which stays out of the repository (its location is
  decided then), and importing other `.idea` settings (run
  configurations, interpreter, code style).
