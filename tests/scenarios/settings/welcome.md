# The Welcome tab: theme and keymap for a new person

## Steps
1. Start Pyoxide with an empty configuration directory
   (`PYOXIDE_CONFIG_DIR=<empty dir>`) on a machine without the reference
   IDE's configuration (`PYOXIDE_REFERENCE_CONFIG=<empty dir>`).
2. Click New Script; close its tab and click it again. Click Open File
   and choose a file.
3. Click "Theme: Islands Dark" at the bottom of the Welcome tab and choose
   Islands Light.
4. Click "Keymap: …" and choose another keymap; press a shortcut that
   differs between the two keymaps.
5. Open the Theme list again and press Escape.
6. Close the Welcome tab; choose Help > Welcome.
7. Open a folder.

## Expected
- Step 1: no first-run window; the main window opens with the "Welcome to
  Pyoxide" tab selected: the title over "Start in one click", the tiles
  New Script (the Python logo), New Notebook, Open File, then Learn and
  Plugins in a second row (New Notebook, Learn and Plugins dimmed, "Not
  available yet" on hover), and at the bottom "Theme: <theme>" and
  "Keymap: <keymap>" with their icons and arrows. New Script has the
  focus: a lighter background and the focus ring. The main toolbar shows no file title. The
  project tool window is "Projects and Files": the Open…, New… and Clone
  Repository… buttons (New… and Clone Repository… dimmed, "Not available
  yet" on hover), a line, the "Search projects" field, a line and "No
  recent projects" a third of the way down; its header has no buttons.
  Open… asks for a folder and opens it as the project.
- Step 2: New Script opens an empty `scratch.py` from `scratches` in the
  configuration folder; the second click opens `scratch_1.py`. Open File
  asks for a file and opens it in a tab.
- Step 3: the list opens above the box with every theme, the current one
  selected in grey (rows of 22); the whole IDE switches at once and the box says "Theme:
  Islands Light" (its icon is the sun in a light theme).
- Step 4: the list holds the keymaps of Settings > Keymap; the keymap is in
  use at once and stored.
- Step 5: the list closes without a change.
- Step 6: the tab closes like any tab and comes back selected.
- Step 7: the folder opens as the project; the Welcome tab goes with the
  other tabs, and the project tool window shows the project's tree.

## Commands
`welcome.open`, `welcome.theme`, `welcome.keymap`, `welcome.new-script`,
`project.open-file`, `menu.down`, `menu.confirm`, `menu.cancel`,
`tabs.close`, `project.open-folder`, `workbench.state` (the tab's `view`,
`projects_and_files`).

## Source
The reference's non-modal Welcome screen (`WelcomeScreenRightTab`,
`WelcomeScreenRightTabComboBoxModel`, `WelcomeScreenLeftPanel` of
`intellij.platform.ide.nonModalWelcomeScreen` in IntelliJ Community at
`pycharm/2026.2.3`); the first start of PyCharm 2026.2.3 without an older
configuration shows it with no Choose Theme or Choose Keymap page.
