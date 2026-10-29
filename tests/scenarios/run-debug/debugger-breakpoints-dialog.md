# Breakpoints dialog

## Steps
1. Set two line breakpoints in two files; press Cmd+Shift+F8 (macOS) or
   Ctrl+Shift+F8.
2. Select a line breakpoint; change its condition; untick Enabled.
3. Select "Any exception".
4. Press + and add a Python Exception Breakpoint for `KeyError`.
5. Double-click a line breakpoint; reopen the dialog, select one and press
   F4; reopen, select one and press Delete.
6. Press Done; restart the project and open the dialog.

## Expected
- Step 1: the dialog "Breakpoints": on the left a tree with check boxes,
  groups "Python Line Breakpoint" (both breakpoints) and "Python
  Exception Breakpoint" ("Any exception"); on the right the first
  breakpoint's properties ("Select a breakpoint" while none is selected).
- Step 2: Enabled, Suspend (All / Thread), Condition, Log options,
  "Remove once hit", "Disable until hitting the following breakpoint";
  edits apply to the gutter icon at once.
- Step 3: Activation policy On termination (selected) / On raise, Ignore
  library files.
- Step 4: the + list shows "Python Exception Breakpoint"; choosing it
  opens the dialog "Select Exception Class" (a name field, the matching
  classes below, Cancel and OK), which stays open. Typing filters the
  classes, Up and Down move, Enter or OK adds the breakpoint: it is listed,
  enabled and selected. Escape or Cancel adds nothing.
- Step 5: double-click and F4 open the source at the line; Delete removes
  the breakpoint and its gutter icon.
- Step 6: everything is kept.

## Commands
`debug.view-breakpoints`, `debug.state` (breakpoints),
`debug.add-exception-breakpoint {"class": ...}`, `dialog.confirm`.

## Source
https://www.jetbrains.com/help/pycharm/using-breakpoints.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `BreakpointsDialog`,
`PyExceptionBreakpointType`, `XDebuggerBundle.properties`. The dialog
layout is to verify against a reference screenshot.
