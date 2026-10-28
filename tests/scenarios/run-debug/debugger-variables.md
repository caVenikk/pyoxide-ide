# Variables pane

## Steps
1. Stop where a list of 1000 integers, a dict, an object with attributes
   and a 10 000-character string are local variables.
2. Expand the list, the dict and the object; scroll to the end of the
   list.
3. Expand "Protected Attributes" of the dict.
4. Select an int and press F2; type `5`, Enter.
5. Select the string and press Cmd+C (macOS) or Ctrl+C.
6. Right-click a variable.
7. Turn off Simplified Variables View (More ⋮ > Debugger Settings).
8. Right-click a container, choose Inspect…; in the dialog expand a child,
   choose Set As Root on it, then Back and Forward.
9. Right-click a value, choose Compare Value with Clipboard….
10. Right-click a value, choose Show Referring Objects….

## Expected
- Step 1: rows `name = {type} value`, sorted by name; containers have an
  expand arrow.
- Step 2: children load on expand ("Collecting data…" while loading);
  the list loads in pages; the object shows its attributes.
- Step 3: the dict shows its items, `__len__` and, last, the Protected Attributes group (the names that start with `_`, collapsed) with the key icon; a frame's own Special Variables group stays at its root.
- Step 4: the value changes in the tree and inline in the editor.
- Step 5: the clipboard holds the value.
- Step 6: New Watch…, Remove Watch, Set Value…, Copy Value, Copy Name,
  Evaluate Expression…, Add to Watches, Jump To Source, in the reference's
  order.
- Step 7: functions, classes and modules appear among the variables.
- Step 8: a window titled "Inspect 'name'" with a toolbar (Set As Root, Back,
  Forward) and the value's tree; the history keeps 11 steps.
- Step 9: a window titled "Clipboard vs Selected Value" with the clipboard
  text ("From clipboard") against the value ("Selected Value").
- Step 10: a window titled "Referring Objects For name" whose children are
  the objects that refer to the value.

## Commands
`debug.set-value {"path": ..., "value": ...}`,
`debug.variables.set-value {"value": ...}` (the selected value),
`debug.variables.copy-value`, `debug.variables.inspect`,
`debug.variables.compare-with-clipboard`,
`debug.variables.show-referring-objects`, `debug.variables.copy-name`,
`debug.variables.jump-to-source`, `debug.variables.context-menu`,
`debug.state` (variables with children), `debug.settings`.

## Source
https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `XVariablesView`,
`XDebuggerBundle.properties`, `PyDebuggerSettings`; debugpy
`variablePresentation` launch setting. The menu order is to verify against
a reference screenshot.
