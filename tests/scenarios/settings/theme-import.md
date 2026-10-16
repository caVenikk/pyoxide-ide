# Import a theme

## Steps
1. In the Theme combo or the gear menu's Theme ▸ choose "Import Theme…".
2. Choose a theme plugin archive that contains a UI theme and an editor
   colour scheme.
3. Import a single `.icls` file the same way.
4. Restart the IDE.
5. Choose a file that is not a theme.

## Expected
- Step 1: a native file chooser for `.theme.json`, `.icls`, `.xml`,
  `.jar`, `.zip` and folders.
- Step 2: the theme appears under "Custom" and is selected at once; an
  import report lists the keys that could not be mapped.
- Step 3: the scheme is offered as an editor scheme.
- Step 4: the imported theme is still listed and selected.
- Step 5: an error message; nothing changes.

## Commands
`settings.import-theme {"path": ...}`, `settings.state` (themes, current
theme).

## Source
https://www.jetbrains.com/help/pycharm/settings-appearance.html;
`docs/theme-formats.md` and the `themes` importer. Pyoxide's "Import
Theme…" replaces the reference's marketplace link.
