# Conditional and non-suspending breakpoints

## Steps
1. In `for i in range(10): total += i` set a breakpoint on the loop body.
2. Right-click the breakpoint; type the condition `i == 3`; press Done.
3. Debug the file.
4. Right-click the breakpoint again, untick Suspend, press Done; debug
   again.
5. Open More (Cmd+Shift+F8 / Ctrl+Shift+F8) and set "Evaluate and log"
   to `i`; debug again.

## Expected
- Step 2: the popup has the title `main.py:<line>`, Enabled, Suspend
  with All / Thread (Thread selected), Make Default, Condition with Python
  completion, "More (Cmd+Shift+F8)" and Done; afterwards the icon has a
  question-mark badge.
- Step 3: the program stops once, with `i = {int} 3`.
- Step 4: the icon becomes the non-suspending one; the program runs to
  the end without stopping.
- Step 5 (logging breakpoints come after the first version): the Console
  shows the value of `i` at each hit.

## Commands
`debug.edit-breakpoint {"file": ..., "line": ..., "condition": ...,
"suspend": ...}`, `debug.current-file`, `debug.state`.

## Source
https://www.jetbrains.com/help/pycharm/using-breakpoints.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`:
`XLightBreakpointPropertiesPanel`, `XBreakpointUIUtil`. The popup layout is
to verify against a reference screenshot.
