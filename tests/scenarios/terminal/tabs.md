# Terminal tabs

## Steps
1. Open the terminal; press Cmd+T (macOS) or Ctrl+Shift+T twice.
2. Double-click the second tab, type `server`, press Enter. Double-click
   it again, clear the field, press Enter.
3. In "server" run `sleep 100`; press Cmd+W / Ctrl+F4 and choose Cancel;
   press it again and choose Terminate.
4. In the third tab type `exit`.
5. Close the remaining tab; press Option+F12 / Alt+F12.
6. In a tab run a program that sets the window title (for example
   `vim`); hover the tab. Rename the tab and run it again.

## Expected
- Step 1: tabs "Local", "Local (2)", "Local (3)"; the new tab is selected
  and has the focus.
- Step 2: a balloon pointing at the tab shows a bold "Session name:" and
  a field with the name selected; Enter renames the tab to "server"; an
  empty name gets a red outline and is not applied.
- Step 3: the dialog "Process 'Terminal server' Is Running" with "Do you
  want to terminate the process 'Terminal server'?", Terminate, Cancel and
  "Don't ask again"; Cancel keeps the tab and the process, Terminate
  closes both.
- Step 4: the tab closes when the shell exits.
- Step 5: closing the last tab hides the tool window; showing it again
  creates a fresh "Local".
- Step 6: while the program runs the tab shows its title, cut in the
  middle to 30 characters, with the full title in the tooltip; a tab the
  user renamed keeps its name.

## Commands
`terminal.new-tab`, `terminal.close-tab`, `terminal.rename-tab
{"name": ...}`, `terminal.type {"text": ...}`, `dialog.confirm`,
`dialog.cancel`, `terminal.state` (tabs, titles), `workbench.state`
(`dialog`).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal` (tab
naming, close session when it ends), `ToolWindowTabRenameActionBase`,
`TerminateRemoteProcessDialog`. The balloon and dialog layout are to verify
against a reference screenshot.
