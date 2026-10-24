# Open files in tabs

## Steps
1. Open a project with `a.py`, `b.py` and `notes.txt`.
2. Open `a.py` from the project tree (double click or Enter).
3. Open `b.py`, then open `a.py` again.
4. Close the selected tab with Cmd+W (macOS) or Ctrl+F4 (Windows, Linux).
5. Close the last tab.

## Expected
- Step 2: a tab `a.py` with a Python file icon appears and is selected; the
  editor shows the file with the caret at line 1, column 1 and has the
  keyboard focus; the status bar shows `1:1`, the line separator and the
  encoding.
- Step 3: `b.py` opens in a second tab to the right; opening `a.py` again
  selects its existing tab instead of opening a second one.
- The middle of the main toolbar shows the selected tab's file: its icon,
  its path relative to the project, the full path in square brackets and
  a chevron; it follows the selected tab and is cut short with an ellipsis
  when the window is narrow.
- Step 4: the selected tab closes and its neighbour becomes selected.
- Step 5: the empty editor area with action hints is shown; the status bar
  no longer shows caret or file widgets.

## Commands
`project.open-file {"path": ...}`, `tabs.close`, `tabs.select {"index": ...}`,
`workbench.state` (`title_path`).

## Source
https://www.jetbrains.com/help/pycharm/managing-editor-tabs.html

# Drop files and folders

## Steps
1. With the project open and no tab open, drag `b.py` and `notes.txt` from
   the file manager onto the editor area and drop them.
2. Drag `a.py` and `b.py` onto the tab strip and drop them.
3. With `a.py` open and a few words of it selected, drag a file over its
   text, then out of the window without dropping it.
4. Drop a file onto a row of the project tree, below the rows, onto the
   toolbar, a stripe and the status bar.
5. Hide the project tool window and drop the project's `src` folder onto the
   editor area; then drop a folder from outside the project.
6. Start the IDE without a project and drop a file, then a folder, onto the
   editor area.
7. Close all tabs and look at the empty editor area with and without a
   project, with the project tool window shown and hidden.

## Expected
- Step 1: `b.py` and `notes.txt` open in tabs in the order they were
  dropped; `notes.txt`, dropped last, is selected and its editor has the
  focus.
- Step 2: `a.py` opens in a new tab; `b.py` is open already, so its tab is
  selected, being dropped last.
- Step 3: nothing opens; the selection and the focus stay as they were.
  (The reference IDE's editor moves its caret to the pointer during any
  drag; that feedback belongs to dragging text inside the editor, which
  keeps the selection while the caret moves.)
- Step 4: nothing happens. Moving and copying files into a folder of the
  tree is a feature of its own.
- Step 5: the project tool window opens, selects `src` and takes the focus;
  the tabs stay as they are. The folder from outside the project is left
  alone (the reference IDE hands it to its other Select In targets, such as
  the system file manager).
- Step 6: the file opens in a tab; the folder opens as the project, as a
  folder dropped on the reference IDE's welcome screen does, and the rest of
  that drop is left alone.
- Step 7: the hints are in a 16 px font, 36 px apart, left-aligned in a
  block centred across the area, the first one at 3/8 of its height, the
  shortcut after the action in its own colour:
  - without a project: `Open a folder to start`, then the hints below;
  - `Search Everywhere Double ⇧` (Double Shift), `Project View ⌘1`
    (Alt+1) while the tool window is hidden, `Go to File ⇧⌘O`
    (Ctrl+Shift+N), `Recent Files ⌘E` (Ctrl+E), `Drop files here to open
    them`; a hint whose action has no shortcut is left out.
  The reference IDE also shows Navigation Bar after Recent Files; it comes
  with that action.

## Commands
`workbench.state` (`tabs`, `active_tab`, `tree`, `project`). A drop has no
command: its result is the same as `project.open-file` for each file in
turn, `project.tree.reveal` for a folder of the project and
`project.open-folder` for a folder without a project.

## Source
IntelliJ Platform Community sources (see NOTICE): `EditorsSplitters.kt`,
`EditorTabbedContainer.kt`, `FileDropManager.kt`, `FileNavigatorImpl.kt`,
`EditorImpl.java`, `ProjectViewDropTarget.java`, `ProjectsTabFactory.kt`,
`DefaultEditorEmptyTextProvider.kt`, `EditorEmptyTextPainter.java`.

# Drag rows from the project tree

## Steps
1. With the project open and no tab open, drag `a.py` from the project tree
   onto the editor area and drop it.
2. Select a few words of `a.py`, drag `notes.txt` from the tree onto its
   text and drop it.
3. Drag `a.py` from the tree onto the tab strip and drop it.
4. Drag the `src` folder from the tree onto the editor area and drop it.
5. Drag `b.py` over the editor area, back over a row of the tree, and drop
   it there.

## Expected
- Step 1: `a.py` opens in a tab and its editor has the focus, as a file
  dropped from the file manager (the reference's project view drags a file
  list, which its editor area opens in the window under the pointer).
- Step 2: `notes.txt` opens in a new tab and is selected; `a.py` keeps its
  selection.
- Step 3: `a.py` is open already, so its tab is selected.
- Step 4: the folder is selected in the tree, which takes the focus (the
  reference navigates to a dropped folder in the project view); the tabs
  stay as they are.
- Step 5: nothing opens; the tree's Move dialog asks where to move `b.py`
  (tests/scenarios/editor/tree-file-operations.md, step 14).
- The reference has no split for a drop from the tree: an editor edge
  splits only for a dragged editor tab (`DockableEditorTabbedContainer`).

## Commands
`workbench.state` (`tabs`, `active_tab`, `tree`). A drag has no command;
its drop is the same as a drop from the file manager.

## Source
IntelliJ Platform Community sources (see NOTICE): `AbstractProjectViewPane.java`
(`MyDragSource`), `EditorsSplitters.kt` (`MyTransferHandler`),
`FileDropHandler.kt`, `FileDropManager.kt`, `DockableEditorTabbedContainer.kt`.
