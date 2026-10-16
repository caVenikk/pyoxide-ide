# Line breakpoints in the gutter

## Steps
1. Open `main.py`. Hover the line numbers of a code line, a blank line,
   a comment, a docstring line and a backslash continuation line.
2. Click the line number of the code line; press Cmd+F8 (macOS) or
   Ctrl+F8 on another code line.
3. Alt+click the first breakpoint; Alt+click it again.
4. Shift+click a third code line.
5. Hover a breakpoint.
6. Click the first breakpoint.
7. Close and reopen the project.

## Expected
- Step 1: on a code line the breakpoint icon replaces the line number
  with a hand cursor and the tooltip "Add Breakpoint"; nothing on the
  other lines.
- Step 2: a red breakpoint icon replaces each line number; the line gets
  the breakpoint background.
- Step 3: the icon turns into the disabled (hollow) one, then back.
- Step 4: a non-suspending logging breakpoint is added (its own icon).
- Step 5: the tooltip shows `main.py:<line>`, the suspend policy and
  "Right-click to edit".
- Step 6: the breakpoint is removed.
- Step 7: the remaining breakpoints are restored with their state.

## Commands
`debug.toggle-breakpoint {"file": ..., "line": ...}`,
`debug.toggle-breakpoint-enabled`, `debug.state` (breakpoints).

## Source
https://www.jetbrains.com/help/pycharm/using-breakpoints.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `PyLineBreakpointType`
(stoppable lines), `BreakpointPromoterEditorListener`,
`XBreakpointUIUtil`, icons in `platform/icons/src/expui/breakpoints/`. The
gutter look is to verify against a reference screenshot.
