# Open the terminal with the project's virtual environment

## Steps
1. Open a uv project that has a `.venv`; press Option+F12 (macOS) or
   Alt+F12 (Windows, Linux).
2. Run `which python` (`where python` on Windows), `echo $VIRTUAL_ENV`
   and `pwd`.
3. Press Option+F12 / Alt+F12 twice.
4. Turn "Activate virtualenv" off in Settings > Tools > Terminal and open
   a new tab.
5. Open a project whose interpreter is a system Python and open the
   terminal.

## Expected
- Step 1: the Terminal tool window opens at the bottom with one tab
  "Local" and the focus in it; the shell is the user's login shell
  (interactive, and a login shell on macOS); the prompt shows the
  environment's prefix, for example `(.venv)` or the uv project name.
- Step 2: python is the environment's interpreter inside `.venv`,
  `VIRTUAL_ENV` points to `.venv`, the working directory is the project
  root.
- Step 3: the first press hides the tool window, the second shows the
  same tab with its output.
- Step 4: the new tab has no environment prefix and no `VIRTUAL_ENV`.
- Step 5: no environment is activated.

## Commands
`workbench.toggle-terminal`, `terminal.new-tab`,
`terminal.type {"text": ...}`, `terminal.state` (tabs, titles, working
directory, visible lines, cursor).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html,
https://www.jetbrains.com/help/pycharm/settings-tools-terminal.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`LocalTerminalStartCommandBuilder`, `LocalOptionsConfigurer`,
`PyVirtualEnvTerminalCustomizer`. The prompt and header look are to verify
against a reference screenshot.
