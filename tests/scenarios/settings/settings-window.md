# Open the Settings window

## Steps
1. Open a project; press Cmd+, (macOS) or Ctrl+Alt+S.
2. Press the shortcut again while the window is open.
3. Click into the main window and type in the editor.
4. Select Editor > Font; press Escape.
5. Open Settings again; resize the window and move the splitter; close
   it and open it again.
6. Open Settings from the toolbar gear menu and from the macOS app menu.

## Expected
- Step 1: one window titled "Settings – <project>" on Appearance (first
  time), the focus in the tree; the tree on the left with a search field,
  the banner with bold breadcrumbs, Back, Forward and Pin Window, the
  bottom bar with "?" and Cancel, Apply, OK on macOS (OK, Cancel, Apply on
  Windows and Linux); Apply disabled.
- Step 2: the same window comes to the front; no second window.
- Step 3: the main window takes keystrokes while Settings stays open
  (non-modal).
- Step 4: Escape without edits closes the window.
- Step 5: it opens on Editor > Font with the size and splitter kept.
- Step 6: both open the same window.

## Commands
`settings.open {"page": ...}`, `settings.select {"page": ...}`,
`settings.cancel`, `settings.state` (page, modified pages, filter,
window).

## Source
https://www.jetbrains.com/help/pycharm/settings-preferences-dialog.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`SettingsNonModalDialog`, `SettingsEditor`, `SettingsTreeView`. The window
layout is to verify against a reference screenshot.
