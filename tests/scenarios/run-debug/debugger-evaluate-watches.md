# Evaluate and watches

## Steps
1. Stop at a breakpoint where `x` and `items` are defined. Select `x + 1`
   in the editor and press Alt+F8.
2. Press Enter; switch to Code Fragment Mode, type two statements,
   press Evaluate; press Close.
3. Press Cmd+Shift+Enter (macOS) or Ctrl+Shift+Enter in the dialog for
   `x + 1`.
4. In the evaluate field of the Variables pane type `len(items)` and
   press Enter; type `items[0]` and press Cmd+Shift+Enter / Ctrl+Shift+Enter.
5. Type `undefined_name` and press Enter.
6. Alt+click `x` in the editor; hover `items` for a second.
7. Stop the session and debug the same configuration again.

## Expected
- Step 1: the "Evaluate" dialog with `x + 1` prefilled.
- Step 2: "Result:" shows `{int} …`; the fragment's last value is shown;
  the expression history keeps the entries.
- Step 3: a watch `x + 1` appears at the top of the Variables pane.
- Step 4: a result node `len(items)` below the watches; the watch
  `items[0]` added.
- Step 5: the error message in red.
- Step 6: Quick Evaluate shows `x`'s value in a popup; after 700 ms the
  value tooltip of `items` opens and can be expanded.
- Step 7: the watches are back for this configuration.

## Commands
`debug.evaluate {"expression": ...}`, `debug.quick-evaluate`,
`debug.new-watch {"expression": ...}`, `debug.remove-watch`,
`debug.state` (watches, results).

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html,
https://www.jetbrains.com/help/pycharm/debugging-code.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`:
`XDebuggerEvaluationDialog`, `XWatchesViewImpl`, `XDebuggerWatchesManager`.
