# 0017. Settings window and application settings

- Status: accepted
- Date: 2026-10-16

## Context
Settings exist only in code and in a few files today: project settings in
`.oxide/project.toml` (shared, ADR 0008) and `.oxide/workspace.toml`
(personal, ADR 0009), and per-feature files in the user's configuration
folder (`find.toml`, `search-everywhere.toml`, `secrets.toml`), each
written by its own module. The theme comes from an environment variable and
the keymap is fixed per platform. #92 asks for the reference's Settings
window (⌘, / Ctrl+Alt+S) with the basic pages; the terminal, debugger and
git add their own pages and application options (ADR 0014-0016).

The reference, 2026.2:
- opens Settings **non-modal** from a project (advanced setting "Show
  settings in a non-modal dialog", on by default): one window per
  application, titled `Settings – <project>`, with a pin toggle; from the
  welcome screen it is modal;
- has a tree of pages with search, a banner with breadcrumbs, "Revert
  changes", Back and Forward, and Cancel / Apply / OK (macOS order
  `Cancel | Apply | OK`, Windows and Linux `OK | Cancel | Apply`); a
  modified page is drawn in the modified colour; Apply errors select the
  page;
- applies a theme at once (combo and quick switcher), not on Apply;
- stores application settings as XML per component in its configuration
  folder (`options/laf.xml`, `keymap.xml`, `editor-font.xml`, `editor.xml`,
  `filetypes.xml`, `jdk.table.xml`, ...).

People who use the reference want their theme, keymap, font and similar
choices in Pyoxide without redoing them, as with exclusions, interpreters
and run configurations.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
platform/platform-impl options (SettingsEditor, SettingsTreeView,
SettingsNonModalDialog, SettingsDialog, ConfigurableEditor, SettingsFilter),
PlatformExtensions.xml, the Python customization of the settings tree; the
reference's settings help pages; the file layout of a reference
configuration folder (data files).

## Options considered
### Storage
1. One file per area in the configuration folder, each module with its own
   load and save (today). Simple, but every module re-implements versions,
   atomic writes and unknown keys, and nothing ties them to a window.
2. One `settings.toml` for scalar options, dedicated files for growing
   data, one store with the rules of `.oxide/project.toml`.
3. A database (SQLite). Not readable or mergeable by people; nothing needs
   queries.

### Window
1. Modal always (`ui::dialog_window` as it is). Blocks the IDE, unlike the
   reference.
2. Non-modal single instance from a main window, modal from a modal flow,
   as the reference.

### Where the window lives
A module of `workbench` (no cycles, but page code of the terminal,
debugger and git UI crates could not reach it), or a `settings-ui` crate
below the workbench that every UI crate can add pages to.

## Decision
Storage option 2, window option 2, a `settings-ui` crate.

### Application store
- `<config dir>/pyoxide/settings.toml` (the configuration folder of
  `dirs::config_dir()`, overridden by `PYOXIDE_CONFIG_DIR` as today),
  `version = 1`, tables by area (`[appearance]`, `[keymap]`,
  `[editor.font]`, `[editor.soft-wraps]`, `[system.files]`, `[terminal]`,
  `[git]`, `[debugger]`, `[import]`). The conventions of ADR 0008 hold:
  `toml_edit`, keys and comments Pyoxide does not know survive, a newer
  version is read and never rewritten, writes replace the file at once,
  defaults are not written.
- Growing data in dedicated files next to it: `interpreters.toml` (#75),
  `themes/<id>.toml` (imported themes in the `themes` format),
  `keymaps/<name>.toml` (ADR 0018). Window and popup state (`find.toml`,
  `search-everywhere.toml`, `settings-window.toml` for size, splitter and
  last page) stays in its own files.
- The `settings` crate (platform, no toolkit) owns the store: loaded once
  at start-up, typed sections with defaults, change events per section,
  background saves that re-read the file and write only the changed keys
  (two windows, or a second instance, do not lose each other's changes).
  Existing per-feature settings move into it when their page is built.
- Scopes: **application** (`settings.toml`), **project shared**
  (`.oxide/project.toml`), **project personal** (`.oxide/workspace.toml`).
  Every option belongs to one scope; pages with project options get the
  reference's project mark.
- Every option is reachable as a command: `settings.get` and
  `settings.set {key, value}` over the store, plus the UI commands below.

### Pages
- A registry of pages: stable dotted id (`appearance`, `editor.font`,
  `python.interpreter`), parent, title, weight (the reference's order),
  scope, search terms (the labels of its options), and a view factory.
- A page edits a draft: `is_modified`, `apply` (may fail with a message
  shown under the page), `reset` ("Revert changes"), and dropping the
  draft on Cancel. Apply applies every modified page; OK is Apply and
  close.
- Exception, as the reference: the theme applies at once (combo and quick
  switcher) and Cancel does not undo it.
- Pages that only edit sections of the store live in `settings-ui`
  (Appearance, Editor > Font, Editor > General); pages that need a
  component's own data live with it (Keymap, System Settings and Editor >
  File Types in `workbench`; Tools > Terminal in `terminal-ui`, and so on).
  Consumers follow the store's change events, not the pages. Products add
  pages through `workbench::Product` (`settings_pages`):
  the Python product adds Python > Interpreter, Type Engine and Tools >
  External Tools, in a top-level "Python" group as the reference.
- Only pages Pyoxide implements are listed. Forms use `ui::form`
  (architecture.md, "Cross-cutting foundations").

### Window
- `settings-ui` (platform, GPUI): tree with search (word prefixes; the
  page name or its search terms; a page with a search of its own, such
  as Keymap, receives the text), banner, bottom bar, the per-OS button
  order, the modified and error marks, Back and Forward, the search
  highlight on the page (the spotlight overlay later).
- From a main window, with or without a project: non-modal, one window per
  application (`ui::dialog_window` gets a non-modal variant), titled
  `Settings – <project>` with a project, `Settings` without. Opening it
  again brings it to the front and navigates to the requested page. On
  activation, pages without edits reload from the current state. Closing a
  project or quitting with edits asks Apply / Don't Save / Cancel.
- From the first-run flow (a modal window before the main window):
  modal, as the reference's welcome screen opens it.
- Commands: `settings.open {page?, filter?}`, `settings.select {page}`,
  `settings.search {text}`, `settings.apply`, `settings.ok`,
  `settings.cancel`, `settings.back`, `settings.forward`,
  `settings.state`; `settings.switch-theme {id?}`,
  `settings.quick-switch`, `settings.import-theme {path}`.

### One-time import from the reference
- At the first start (no `settings.toml`), the first-run flow offers the
  newest reference configuration folder found ("Import from …" or "Skip
  Import"), then theme and keymap choices with the imported ones
  preselected. The decision is recorded in `[import]` of `settings.toml`
  (source, version, time and snapshot, or `skipped`) and never asked
  again; `settings.import-reference` repeats it on request.
- The import keeps a snapshot of the reference's data files in the
  configuration folder (`imported/<source>-<version>/`; never
  `security.xml`, password stores or token files). A feature built after
  the import takes its keys from the snapshot once, at its first start,
  and is listed in `[import.applied]`; values changed later are never
  overwritten.
- Read once, data files only: `laf.xml` and `colors.scheme.xml` (+ the
  named `colors/*.icls`) for the theme and scheme, `keymap.xml` (+ the
  active `keymaps/*.xml`), `editor-font.xml`, `editor.xml` (soft wraps),
  `ui.lnf.xml` (the options Pyoxide honours), `ide.general.xml`
  (autosave), `filetypes.xml` (associations), `jdk.table.xml`
  (interpreters that still exist). The reference's files are never written.
- A theme from a plugin is named in `laf.xml` by an id declared only in the
  plugin's descriptor. Plugin archives in the reference's plugin folder
  may be opened to read `META-INF/plugin.xml` (`themeProvider` id and path)
  and the theme and scheme files it names; `.class` files and anything
  executable are never opened (docs/clean-room.md).
- The two copies of the reference's configuration-folder lookup
  (`secrets::intellij` and the Python product's interpreter code) become
  one helper in `settings-import`.

## Consequences
- New crates `settings` and `settings-ui`; `workbench` depends on
  `settings-ui` and registers its pages; UI crates of the terminal,
  debugger and git register theirs.
- Settings of different areas share one file and one save path; a broken
  `settings.toml` falls back to defaults with a notification and is not
  overwritten until the person changes a setting.
- On acceptance, `docs/clean-room.md` names plugin descriptors
  (`META-INF/plugin.xml`) among the data resources that may be extracted.
- No two-way sync: changes in the reference after the import do not reach
  Pyoxide.
- Not in the first version: the spotlight overlay, the pin toggle, the
  reference's advanced settings page, Plugins, Version Control and
  Languages & Frameworks pages, the packages table (an issue of its own).
