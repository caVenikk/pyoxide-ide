# The Welcome tab: theme and keymap for a new person

## Steps
1. Start Pyoxide with an empty configuration directory
   (`PYOXIDE_CONFIG_DIR=<empty dir>`) on a machine without the reference
   IDE's configuration (`PYOXIDE_REFERENCE_CONFIG=<empty dir>`).
2. Click "Theme: Islands Dark" at the bottom of the Welcome tab and choose
   Islands Light.
3. Click "Keymap: …" and choose another keymap; press a shortcut that
   differs between the two keymaps.
4. Open the Theme list again and press Escape.
5. Close the Welcome tab; choose Help > Welcome.
6. Open a folder.

## Expected
- Step 1: no first-run window; the main window opens with the "Welcome to
  Pyoxide" tab selected: the title over "Start in one click", the Open
  File tile, and at the bottom "Theme: <theme>" and "Keymap: <keymap>"
  with their icons and arrows. The main toolbar shows no file title. The
  project tool window is "Projects and Files": the Open…, New… and Clone
  Repository… buttons (New… and Clone Repository… dimmed, "Not available
  yet" on hover), a line, the "Search projects" field, a line and "No
  recent projects" a third of the way down; its header has no buttons.
  Open… asks for a folder and opens it as the project.
- Step 2: the list opens above the box with every theme, the current one
  selected; the whole IDE switches at once and the box says "Theme:
  Islands Light" (its icon is the sun in a light theme).
- Step 3: the list holds the keymaps of Settings > Keymap; the keymap is in
  use at once and stored.
- Step 4: the list closes without a change.
- Step 5: the tab closes like any tab and comes back selected.
- Step 6: the folder opens as the project; the Welcome tab goes with the
  other tabs, and the project tool window shows the project's tree.

## Commands
`welcome.open`, `welcome.theme`, `welcome.keymap`, `menu.down`,
`menu.confirm`, `menu.cancel`, `tabs.close`, `project.open-folder`,
`workbench.state` (the tab's `view`, `projects_and_files`).

## Source
The reference's non-modal Welcome screen (`WelcomeScreenRightTab`,
`WelcomeScreenRightTabComboBoxModel`, `WelcomeScreenLeftPanel` of
`intellij.platform.ide.nonModalWelcomeScreen` in IntelliJ Community at
`pycharm/2026.2.3`); the first start of PyCharm 2026.2.3 without an older
configuration shows it with no Choose Theme or Choose Keymap page.
