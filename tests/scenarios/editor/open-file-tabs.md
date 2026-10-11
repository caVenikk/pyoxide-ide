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
- Step 4: the selected tab closes and its neighbour becomes selected.
- Step 5: the empty editor area with action hints is shown; the status bar
  no longer shows caret or file widgets.

## Commands
`project.open-file {"path": ...}`, `tabs.close`, `tabs.select {"index": ...}`,
`workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/managing-editor-tabs.html
