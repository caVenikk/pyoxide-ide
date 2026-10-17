# One-time pick-up of the reference IDE's settings

## Steps
1. Start Pyoxide with an empty configuration directory
   (`PYOXIDE_CONFIG_DIR=<empty dir>`) on a machine where the reference
   IDE's configuration has a non-default theme and scheme (a theme
   plugin), editor font size 14 with ligatures on, soft wraps off, a
   custom keymap based on macOS and two interpreters (one of them deleted
   from disk).
2. Choose the newest configuration ("Import Settings", the folder's button
   names its version), then the preselected theme and keymap, and
   Continue.
3. Look at the theme, an editor, soft wraps, a few shortcuts of the
   custom keymap and the interpreter list.
4. Look at `settings.toml` and `imported/` in the configuration directory.
5. Restart Pyoxide.
6. Start once more with an empty configuration and choose "Skip Import".

## Expected
- Step 1: the first-run window comes before the main window, modal, on
  "Import Settings": the newest configuration found, "Skip Import", and
  "Other Options" with the older ones and "Config or Installation
  Directory…".
- Step 2: "Importing settings…", then "Choose Theme" and "Choose Keymap"
  preselect the imported values; Continue on "Choose Keymap" opens the
  main window with a "Settings imported" balloon naming what came in and
  what could not.
- Step 3: theme, scheme, font, ligatures, soft-wrap setting and keymap
  match the reference; only the interpreter that still exists is listed
  (the configured interpreter list, #75, takes it from the snapshot).
- Step 4: `[import]` has `source`, `version`, `time` and
  `snapshot = "imported/<source>-<version>"`; `[import.applied]` lists the
  importers that ran. The snapshot holds the data files (`options/*.xml`,
  `keymaps/`, `colors/`, `codestyles/`, `templates/`), never
  `security.xml`, `c.kdbx`, `c.pwd`, licences or tokens, and no plugins.
- Step 5: no second import; an area added in a newer version takes its
  part from the snapshot once and is added to `[import.applied]`.
- Step 6: defaults (Islands theme, the platform's keymap); `[import]`
  has `source = "skipped"` and no snapshot is kept.

## Commands
`first-run.state`, `first-run.import`, `first-run.skip`,
`first-run.continue`, `settings.import-reference`, `settings.get`.

## Source
https://www.jetbrains.com/help/pycharm/sharing-your-ide-settings.html;
the reference's data files `laf.xml`, `colors.scheme.xml`, `keymap.xml`,
`editor-font.xml`, `editor.xml`, `ui.lnf.xml`, `ide.general.xml`,
`filetypes.xml`, `jdk.table.xml` and theme plugin descriptors (formats
read in IntelliJ Community at `pycharm/2026.2.3`); the first-run flow of
`intellij.ide.startup.importSettings` at the same tag.
