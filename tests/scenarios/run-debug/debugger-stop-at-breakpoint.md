# Stop at a breakpoint

## Steps
1. `main.py`:
   ```python
   def f(a):
       b = a + 1
       return b

   print("start")
   f(1)
   ```
   Set a breakpoint on `b = a + 1`.
2. Press Ctrl+D (macOS) or Shift+F9 with `main.py` in the editor.
3. Look at the Frames and Variables panes, the editor and the Console tab.
4. Press F9.

## Expected
- Step 2: the Debug tool window opens with a tab "main"; the Console
  shows the command line and "start".
- Step 3: "Threads & Variables" is selected; the thread combo shows
  `MainThread`; frames `f, main.py:2` (selected) and `<module>,
  main.py:6`; variables `a = {int} 1`; the execution line is highlighted
  in the editor; inline value `a: 1` after the `def` line.
- Step 4: the program finishes; the Console tab is selected and ends with
  "Process finished with exit code 0"; the toolbar offers Rerun.

## Commands
`debug.current-file`, `debug.resume`, `debug.state` (session state,
threads, frames, variables, execution point), `run.state` (console).

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window.html,
https://www.jetbrains.com/help/pycharm/debugging-code.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `XDebugSessionTab`,
`XFramesView`, `XVariablesView`, `ExecutionPointHighlighter`,
`InlineDebugRenderer`, `PyStackFrame`. Where the focus goes on the stop
is to verify against a reference screenshot.
