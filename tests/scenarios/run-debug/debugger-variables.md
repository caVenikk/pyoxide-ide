# Variables pane

## Steps
1. Stop where a list of 1000 integers, a dict, an object with attributes
   and a 10 000-character string are local variables.
2. Expand the list, the dict and the object; scroll to the end of the
   list.
3. Expand "Special Variables".
4. Select an int and press F2; type `5`, Enter.
5. Select the string and press Cmd+C (macOS) or Ctrl+C.
6. Right-click a variable.
7. Turn off Simplified Variables View (More ⋮ > Debugger Settings).

## Expected
- Step 1: rows `name = {type} value`, sorted by name; containers have an
  expand arrow.
- Step 2: children load on expand ("Collecting data…" while loading);
  the list loads in pages; the object shows its attributes.
- Step 3: the dunder names are inside this group.
- Step 4: the value changes in the tree and inline in the editor.
- Step 5: the clipboard holds the value.
- Step 6: New Watch…, Remove Watch, Set Value…, Copy Value, Copy Name,
  Evaluate Expression…, Add to Watches, Jump To Source, in the reference's
  order.
- Step 7: functions, classes and modules appear among the variables.

## Commands
`debug.set-value {"path": ..., "value": ...}`, `debug.copy-value`,
`debug.state` (variables with children), `debug.settings`.

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `XVariablesView`,
`XDebuggerBundle.properties`, `PyDebuggerSettings`; debugpy
`variablePresentation` launch setting. The menu order is to verify against
a reference screenshot.
