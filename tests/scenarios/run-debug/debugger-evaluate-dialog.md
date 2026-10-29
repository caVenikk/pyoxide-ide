# Evaluate dialog

## Steps
1. Stop at a breakpoint where `x`, `items` and `summary` are defined.
   Select `summary` in the Variables pane and press Alt+F8 (Evaluate
   Expression… in the Run menu).
2. Press Enter.
3. Press the down arrow of the field (Show History), choose an entry.
4. Type `undefined_name`, press Enter.
5. Press Alt+M (or the expand button in the field), type
   `y = x + 1` and `y * 2` on two lines, press Cmd+Enter (macOS) or
   Ctrl+Enter.
6. Press the collapse button; press Cmd+Shift+Enter / Ctrl+Shift+Enter.
7. Press Alt+R, then the arrow keys.
8. Resume the program and stop at the breakpoint again; press Evaluate.
9. Press Escape; open the dialog again; stop the session.

## Expected
- Step 1: a window "Evaluate" opens over the IDE (the IDE keeps its
  input), "Expression:" with `summary` selected in the field, the expand
  and history buttons inside it, "Use ⇧⌘⏎ to add to Watches" under it,
  "Result:" empty, Close and Evaluate at the bottom.
- Step 2: "Result:" shows `result = {dict: 2} {'count': 4, 'sum': 0}`,
  expanded, with the items and Special Variables; the first row is
  selected; the Variables pane reads its values again.
- Step 3: the list of the expressions evaluated before, the latest
  first, as wide as the field right under it; the entry replaces the
  text.
- Step 4: the adapter's message with the error icon in the result.
- Step 5: the code fragment mode: "Code fragment:" with a multi-line
  editor and a collapse button over "Result:"; Enter adds a line; the
  result is the value of the last statement.
- Step 6: back in the expression mode (the text on one line); the
  expression is a new watch at the top of the Variables pane; the dialog
  stays open.
- Step 7: the focus moves to the result tree; up, down, left and right
  move in it.
- Step 8: the dialog evaluates in the new frame; Escape closes it (a
  history list first); stopping the session closes it.
- The mode chosen last is the one the dialog opens in next time.

## Commands
`debug.evaluate-expression {"expression": ...}` opens the dialog or puts
the expression into the open one; in the dialog's window
`evaluate-dialog.evaluate {"expression": ...}`, `.switch-mode`,
`.add-to-watches`, `.focus-result`, `.history`, `.choose-history`,
`.close`; `debug.state` (`evaluate_dialog`: the mode, the texts, the
result's state, message and tree, the open history list).

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`XDebuggerEvaluationDialog`, `ExpressionInputComponent`,
`CodeFragmentInputComponent`, `XDebuggerEvaluateActionHandler`.
