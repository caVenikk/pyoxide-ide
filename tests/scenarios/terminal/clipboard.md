# Select, copy, paste and clear in the terminal

## Steps
1. Print a few lines; drag over part of them; press Cmd+C (macOS) or
   Ctrl+C; double-click a word; triple-click a line; Shift+Alt drag a
   rectangle.
2. Run `sleep 10`; with no selection press Ctrl+C.
3. Press Cmd+V / Ctrl+V at the prompt; middle-click in the output.
4. Press Cmd+A (macOS).
5. Fill the scrollback, then press Cmd+K (macOS) or choose Clear Terminal
   in the output context menu.
6. Right-click in the output with and without a selection.

## Expected
- Step 1: the selection follows the drag; Copy puts exactly the selected
  text on the clipboard; double click selects a word, triple click a
  line, Shift+Alt drag a rectangle.
- Step 2: without a selection Ctrl+C goes to the shell and interrupts
  `sleep`.
- Step 3: the clipboard text appears at the prompt; middle click pastes
  too ("Paste on middle mouse button click", on).
- Step 4: all output is selected.
- Step 5: the scrollback and the screen are cleared; the prompt line
  stays at the top.
- Step 6: Find, New Tab, Close Tab; Copy (disabled without a selection),
  Paste, Paste from History…, Select All; Clear Terminal, Page Up, Page
  Down, Line Up, Line Down.

## Commands
`terminal.copy`, `terminal.paste`, `terminal.select-all`,
`terminal.clear`, `terminal.state` (selection, visible lines).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal` actions
and the `Terminal.ReworkedTerminalContextMenu` group. The context menu
look is to verify against a reference screenshot.
