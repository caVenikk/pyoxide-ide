# The terminal on Windows

## Steps
1. On Windows 10 1809 or later open a project with a `.venv`; press
   Alt+F12.
2. Run `python -c "import sys; print(sys.prefix)"` and `cd`.
3. Open a Command Prompt tab from the chevron popup; run `echo
   %VIRTUAL_ENV%`.
4. Run `dir` in a folder with long and non-ASCII names; resize the tool
   window while it prints.
5. Close the tab while `ping -t localhost` runs.

## Expected
- Step 1: `powershell.exe` runs through ConPTY; the environment's
  `Activate.ps1` was run before the first prompt, which shows its prefix.
- Step 2: the environment's prefix and the project root, with
  backslashes and without a `\\?\` prefix.
- Step 3: Command Prompt has no shell integration but gets `VIRTUAL_ENV`
  and `PATH` from the activate script.
- Step 4: names render correctly; the layout follows the new size.
- Step 5: the close confirmation appears; Terminate ends `ping` and the
  shell.

## Commands
`workbench.toggle-terminal`, `terminal.new-tab {"shell": ...}`,
`terminal.type {"text": ...}`, `terminal.close-tab`, `terminal.state`.

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html,
https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`PyVirtualEnvTerminalCustomizer`, `LocalTerminalStartCommandBuilder`.
