# Search in Settings

## Steps
1. Open Settings; with the focus in the tree type `wrap`.
2. Press Down twice, then Up twice; press Enter.
3. Press Escape.
4. Type `zzzz`.
5. Press Escape twice.
6. Open Settings and press Cmd+F (macOS) or Ctrl+F.

## Expected
- Step 1: the typing goes to the search field; after a short delay the
  tree keeps Editor > General (and its parents) and pages whose options
  mention wraps; the first match is selected; on the page the matching
  labels are highlighted.
- Step 2: Up and Down move the tree selection while the focus stays in
  the field; Enter moves the focus to the page.
- Step 3: the first Escape clears the filter; the window stays.
- Step 4: no page matches; the field turns red.
- Step 5: the first Escape clears the filter, the second closes the
  window.
- Step 6: the focus goes to the search field.

## Commands
`settings.search {"text": ...}`, `settings.state` (filter, visible pages,
selected page).

## Source
https://www.jetbrains.com/help/pycharm/settings-preferences-dialog.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `SettingsFilter`,
`SettingsTreeView`. The highlight on the page (spotlight) is a later
feature and to verify against a reference screenshot.
