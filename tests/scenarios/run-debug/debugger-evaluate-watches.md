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
6. Click the field's chevron; press Down, then Enter.
7. Right-click the watch `items[0]`: Pause Watch; Step Over; click
   "… Evaluate" in its row; right-click it again: Resume Watch.
8. Right-click an integer value: View as > Hex; then Jump To Type Source
   on a value of a class of the project.
9. Right-click any row: Remove All Watches.
10. Alt+click `x` in the editor; hover `items` for a second.
11. Stop the session and debug the same configuration again.

## Expected
- Step 1: the "Evaluate" dialog with `x + 1` prefilled.
- Step 2: "Result:" shows `{int} …`; the fragment's last value is shown;
  the expression history keeps the entries.
- Step 3: a watch `x + 1` appears at the top of the Variables pane.
- Step 4: a result node `len(items)` below the watches; the watch
  `items[0]` added.
- Step 5: the error in red, `{NameError}NameError("name 'undefined_name'
  is not defined")`, with the error icon; `undefined_name` has a red wave
  in the field and the field a red outline (also when it loses the focus).
- Step 6: the expressions evaluated before, the latest first, in a list as
  wide as the field and right under it, no row highlighted; Down
  highlights the first, Enter puts it into the field and closes the list.
- Step 7: the watch keeps its value with the pause icon; after the step
  its row says "The watch is paused … Evaluate" and it is not evaluated;
  "… Evaluate" shows its value once; resumed, it is evaluated again. The
  watch stays paused in the next session.
- Step 8: the value as `0x…`, View as with Decimal checked before; the
  class's declaration opens (nothing for a builtin type).
- Step 9: the watches and the result go.
- Step 10: Quick Evaluate shows `x`'s value in a popup; after 700 ms the
  value tooltip of `items` opens and can be expanded.
- Step 11: the watches are back for this configuration.

## Commands
`debug.evaluate {"expression": ...}`, `debug.quick-evaluate`,
`debug.variables.evaluate {"expression": ...}` (the evaluate field),
`debug.variables.watch-expression`, `debug.variables.history`,
`debug.variables.choose-history`, `debug.variables.editor-history`,
`debug.new-watch {"expression": ...}`, `debug.remove-watch`,
`debug.remove-all-watches`, `debug.pause-watch`,
`debug.evaluate-paused-watch`, `debug.edit-watch`, `debug.add-to-watches`,
`debug.variables.view-as`, `debug.variables.jump-to-type-source`,
`debug.state` (watches and the paused ones, results, the field, its
unresolved names and its history, the open history list).

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html,
https://www.jetbrains.com/help/pycharm/debugging-code.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`:
`XDebuggerEvaluationDialog`, `XWatchesViewImpl`, `XDebuggerWatchesManager`,
`XPauseWatchAction`, `XDebuggerExpressionComboBox`, `ViewAsGroup`.
