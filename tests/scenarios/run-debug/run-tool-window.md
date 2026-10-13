# The Run tool window

## Steps
1. In a new project press Cmd+4 (macOS) or Alt+4 (Windows, Linux) before
   anything ran; look at the left stripe.
2. Run `main.py`, look at the left stripe while it runs and after.
3. Run `a.py` after `main.py` ended; run it again while `a.py` runs; pin
   a tab from its context menu and run again.
4. Press Cmd+W (macOS) or Ctrl+F4 in the Run tool window while the
   process of its tab runs; repeat and choose Disconnect.
5. Press Escape in the console, then Shift+Escape.
6. Open Recent Files (Cmd+E / Ctrl+E).
7. Drag the gap above the Run tool window.

## Expected
- Step 1: the Run tool window opens with "To run your code, do one of the
  following:" and its three lines; its stripe button appears.
- Step 2: the Run button sits at the top of the left stripe's bottom
  group and carries a green dot while a process runs; the tab's Python
  icon carries the dot too, and is grey after the process ended; the
  tab's tooltip is "Process ID: N" while it runs.
- Step 3: the ended "main" tab is reused for "a"; a running tab is not
  reused (a new tab opens); a pinned tab is kept and a new tab opens.
- Step 4: the dialog "Process 'main' Is Running" with a warning icon,
  "Do you want to terminate the process 'main'?", a "Don't ask again" check
  box and [Cancel] [Disconnect] [Terminate] (the default). Terminate stops
  the process and closes the tab; Disconnect closes the tab and leaves the
  process running. Closing the last tab hides the tool window.
- Step 5: Escape moves the focus to the editor; Shift+Escape hides the
  tool window.
- Step 6: Recent Files lists the Run tool window next to Project.
- Step 7: the tool window spans under the project tool window and the
  editor and follows the mouse; a third of the window high by default.

## Commands
`workbench.toggle-run`, `workbench.hide-tool-window`,
`workbench.focus-editor`, `run.close-tab`, `run.close-other-tabs`,
`run.close-all-tabs`, `run.select-tab`, `run.toggle-pin`, `run.state`.

## Source
https://www.jetbrains.com/help/pycharm/run-tool-window.html; observed in
PyCharm 2026.2.3; IntelliJ Platform Community (Apache-2.0)
`RunContentManagerImpl`, `ActivateRunToolWindowAction`,
`ToolWindowPane`.
