# Stop, rerun, pause and mute

## Steps
1. Debug a script that loops forever and reads `input()` in each pass.
2. Type a line in the Console tab and press Enter.
3. Press Pause Program in the toolbar.
4. Press F9.
5. Press Cmd+R (macOS) or Ctrl+F5.
6. Press Mute Breakpoints; press it again.
7. Press Cmd+F2 / Ctrl+F2; list the system's Python processes.
8. Debug again and close the session tab instead of pressing Stop.

## Expected
- Step 2: the program receives the line (stdin works in the Console).
- Step 3: all threads are suspended where they are; frames and variables
  are shown; the editor jumps to the line; Pause is disabled, Resume and
  the steps are enabled.
- Step 4: the program runs again.
- Step 5: the session stops and starts again in the same tab.
- Step 6: every breakpoint icon turns grey (muted) and no breakpoint
  stops the program; unmuting restores them.
- Step 7: the session ends with "Process finished with exit code …"; no
  adapter, launcher or program process is left.
- Step 8: closing the tab stops the session the same way.

## Commands
`debug.pause`, `debug.resume`, `run.rerun`, `run.stop`,
`debug.mute-breakpoints`, `debug.state` (session state, processes).

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `XDebugSessionTab`,
`ExecutionActions.xml`; DAP `pause`, `terminate`, `disconnect`.
