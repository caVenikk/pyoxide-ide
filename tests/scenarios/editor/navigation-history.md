# Navigation history: Back and Forward

The reference IDE's Back and Forward with its default keymaps: macOS
("Mac") and Windows/Linux ("Win"). Covered by
`crates/platform/workbench/tests/navigation.rs`, the model's unit tests in
`crates/platform/workbench/src/navigation.rs` and
`crates/platform/workbench/tests/keymap.rs` (every binding).

## Keys
| Action | Mac | Win |
|---|---|---|
| Back | Cmd+[, Cmd+Alt+Left, mouse back button | Ctrl+Alt+Left, mouse back button |
| Forward | Cmd+], Cmd+Alt+Right, mouse forward button | Ctrl+Alt+Right, mouse forward button |

## Steps
1. Open `a.py`, put the caret on line 10; open `b.py`, caret on line 5;
   open `c.py`. Press Back twice, then Forward twice.
2. In `a.py` press Cmd+B (Ctrl+B) on a name declared 90 lines up, then
   on one declared in `b.py`. Press Back twice, Forward once.
3. In a file: Go to Line (Cmd+L / Ctrl+G) to a far line; click on a line
   far from the caret; Cmd+End and Cmd+Home (Ctrl+End, Ctrl+Home); F2 to
   the next problem; Cmd+G (F3) with the caret in the editor after a
   search. Press Back after each.
4. Move the caret with the arrow keys, type, select with Shift+click or
   Shift+Cmd+End, press Enter in the find bar's field. Press Back.
5. Jump twice to places less than four lines apart, press Back.
6. Go back once, then open another file.
7. With three tabs open, close the active one; close another one; close
   the last one.
8. Close a file's tab, delete the file, press Back.
9. Scroll so that the caret's line is near the top of the view, open
   another file, press Back.
10. Open another folder as the project, press Back.

## Expected
- Step 1: Back shows `b.py` with the caret on line 5, then `a.py` on line
  10; Forward shows `b.py`, then `c.py`. Back with nothing to go back to
  (and Forward with nothing to go forward to) does nothing.
- Step 2: Back goes to the declaration in `a.py`, then to the line where
  the first Cmd+B was pressed; Forward to the declaration.
- Step 3: each of these is a navigation: Back returns to the caret's place
  before it.
- Step 4: none of these is a navigation: Back goes to the place before.
- Step 5: places in the same file less than four lines apart count as one
  place: one Back goes past both.
- Step 6: Forward has nothing to go to any more.
- Step 7: closing the active tab: Back opens its file again with the caret
  where it was. Closing a tab that is not the active one: the active
  editor's place is recorded (the reference selects the active tab
  again), and Forward has nothing more. Closing the last tab records
  nothing.
- Step 8: the deleted file's places are skipped.
- Step 9: the caret's line comes back at the same height in the view.
- Step 10: the history is empty.

## Commands
`workbench.back`, `workbench.forward`, `workbench.state` (`navigation`:
the places of Back and Forward, the next one last).

## Notes
- Recent Locations (Cmd+Shift+E), Last Edit Location and the toolbar's
  Back and Forward buttons are features of their own.
- Pyoxide keeps the primary caret of a place; the reference keeps all
  carets and selections.

## Source
IntelliJ Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`IdeDocumentHistoryImpl.kt`, `TextEditorState.java`, `EditorTabbedContainer.kt`,
`EditorWindow.kt`, `JBTabsImpl.kt`, the callers of
`includeCurrentCommandAsNavigation` (`EditorImpl.java`,
`EditorActionUtil.java`, `GotoLineAction.java`, `GotoNextErrorHandler.java`,
`SearchAgainAction.java`, `FindUtil.java`, `ide.kt`), `registry.properties`
(`editor.navigation.history.stack.size`) and the keymaps (`Back`,
`Forward`); https://www.jetbrains.com/help/pycharm/navigating-through-the-source-code.html.
