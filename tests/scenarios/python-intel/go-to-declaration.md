# Go to declaration and usages

## Steps
1. Hold Cmd (Ctrl) and move the mouse over a function call.
2. Cmd+click it; press Cmd+[ (Back).
3. Put the caret on the call and press Cmd+B (Ctrl+B).
4. Press Cmd+B on the function's own name in its `def` line.
5. Press Cmd+B on a standard library or third-party name.
6. Press Cmd+B on a name with no declaration.
7. Press Cmd+Alt+F7 (Ctrl+Alt+F7) on a name; press Alt+F7.

## Expected
- Step 1: the name is underlined in the link colour with a hand cursor.
- Step 2: the declaring file opens at the declaration; Back returns.
- Step 3: same as the click.
- Step 4: the Show Usages popup lists the usages (file, line, code).
- Step 5: the library file opens read-only (stub files from a cache
  directory as well).
- Step 6: a hint "Cannot find declaration to go to".
- Step 7: the Show Usages popup; Alt+F7 shows the usages in the Find tool
  window (later than the first version).

## Commands
`code.go-to-declaration`, `code.show-usages`, `code.find-usages`,
`editor.state` (`link_highlight`).

## Source
https://www.jetbrains.com/help/pycharm/navigating-through-the-source-code.html,
https://www.jetbrains.com/help/pycharm/find-highlight-usages.html; IntelliJ
Platform Community (Apache-2.0) keymaps at `pycharm/2026.2.3`
(`GotoDeclaration`, `ShowUsages`, `FindUsages`).
