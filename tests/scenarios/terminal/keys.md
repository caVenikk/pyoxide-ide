# Keys in the terminal

## Steps
1. With the focus in a terminal tab press Cmd+1 (macOS) or Alt+1, then
   go back to the terminal.
2. Press Shift+Escape; press Option+F12 / Alt+F12.
3. Press Cmd+Shift+A (macOS) or Ctrl+Shift+A; press Escape.
4. On Windows or Linux press Ctrl+E.
5. Start `vim` and press Escape inside it.
6. On macOS type Option+B with "Use Option as Meta key" off, then on.
7. Turn "Override IDE shortcuts" off and press Ctrl+R (bash reverse
   search) and Cmd+1 / Alt+1.

## Expected
- Step 1: the Project tool window is toggled (an allowed IDE action).
- Step 2: Shift+Escape hides the terminal; Option+F12 / Alt+F12 shows it
  with the focus in the session.
- Step 3: Find Action opens; Escape closes it.
- Step 4: Ctrl+E reaches the shell (end of line), not Recent Files.
- Step 5: vim receives Escape.
- Step 6: off, Option+B types the composed character; on, the shell gets
  ESC b (word back).
- Step 7: with the option off, IDE shortcuts win over the shell.

## Commands
`terminal.type {"keys": ...}`, `terminal.state` (visible lines),
`workbench.state` (focused part, tool windows). Keymap checks belong in the
workbench keymap tests.

## Source
https://www.jetbrains.com/help/pycharm/settings-tools-terminal.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`TerminalEventDispatcher` (allowed IDE actions), keymaps in
`platform/platform-resources/src/keymaps/`.
