# One-time pick-up of the reference IDE's settings

## Steps
1. Start Pyoxide with an empty configuration directory on a machine where
   the reference IDE's configuration has a non-default theme and scheme,
   editor font size 14 with ligatures on, soft wraps off, a custom keymap
   based on macOS and two interpreters (one of them deleted from disk).
2. Choose "Import from PyCharm 2026.2", then the preselected theme and
   keymap, and Continue.
3. Look at the theme, an editor, soft wraps, a few shortcuts of the
   custom keymap and the interpreter list.
4. Restart Pyoxide.
5. Start once more with an empty configuration and choose "Skip Import".

## Expected
- Step 2: the first-run window offers the configurations found, newest
  first, and "Skip Import"; "Choose Theme" and "Choose Keymap" preselect
  the imported values.
- Step 3: theme, scheme, font, ligatures, soft-wrap setting and keymap
  match the reference; only the interpreter that still exists is listed.
- Step 4: no second import; the settings file carries the done-once
  marker.
- Step 5: defaults (Islands theme, the platform's keymap).

## Commands
`settings.import-reference`, `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/sharing-your-ide-settings.html;
the reference's data files `laf.xml`, `colors.scheme.xml`, `keymap.xml`,
`editor-font.xml`, `editor.xml`, `jdk.table.xml` (formats read in
IntelliJ Community at `pycharm/2026.2.3`). The first-run window is a later
feature; the import itself is in the first version.
