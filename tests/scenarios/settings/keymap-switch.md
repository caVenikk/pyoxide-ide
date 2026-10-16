# Switch the keymap

## Steps
1. On macOS open Settings > Keymap; open the keymap combo.
2. Choose "Windows" and press Apply; close Settings.
3. Press Ctrl+Alt+S, then Cmd+,; open a main menu.
4. In the toolbar gear menu choose Keymap ▸ macOS.
5. On the Keymap page type `comment` in the search field; then use Find
   Actions by Shortcut and press Cmd+/.
6. Choose "VSCode (macOS)" and Apply.

## Expected
- Step 1: the predefined keymaps "macOS", "Windows", "GNOME", "KDE",
  "XWin", "VSCode", "VSCode (macOS)" (predefined names in bold).
- Step 2: the bindings change after Apply.
- Step 3: Ctrl+Alt+S opens Settings, Cmd+, does nothing; the menus show
  the new shortcuts.
- Step 4: the switch happens at once; the menus follow.
- Step 5: the actions tree shows only the matching actions with their
  shortcuts; the shortcut search finds Comment with Line Comment.
- Step 6: VS Code bindings are active (for example Cmd+P opens Go to
  File).

## Commands
`settings.switch-keymap {"name": ...}`, `settings.search {"text": ...}`,
`settings.state` (keymap), `workbench.state` (bindings).

## Source
https://www.jetbrains.com/help/pycharm/settings-keymap.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `KeymapPanel`,
`QuickChangeKeymapAction`, keymaps in `platform/platform-resources/src/keymaps/`
and `plugins/keymaps/vscode-keymap`. The page layout is to verify against
a reference screenshot.
