# Files changed outside the IDE

## Steps
1. Open `app.py` in a tab, put the caret on a line in the middle, then
   change the file with another program (add two lines at the top, change
   the caret's line) while the IDE window has the focus.
2. Switch to another application and back.
3. Undo in the editor; answer the question; Redo.
4. Type into `app.py` without saving, then change the file with another
   program and switch to another application and back.
5. Repeat step 4 and choose the other button.
6. Type into `app.py`, change the file with another program and press
   Save All (Cmd+S on macOS, Ctrl+S elsewhere) before switching away.
7. Delete an open file whose editor has no changes with another program,
   then switch away and back.
8. Type into an open file, delete it with another program, switch away
   and back, then close its tab.
9. Rename an open file with another program.
10. Create, change and delete files in the project with another program
    (`git checkout` of another branch); also Replace in Files.
11. Reload All from Disk (File menu; Cmd+Alt+Y on macOS, Ctrl+Alt+Y
    elsewhere).
12. Type into a file and quit at once.

## Expected
- Step 1: with the file watcher, the editor shows the new text at once;
  otherwise when the window gets the focus (step 2, "Synchronize files on
  frame activation", on by default). The caret stays on its line (now
  two lines lower) and column; the view does not scroll; the tab is not
  modified.
- Step 3: a question "Undo Reload from Disk?" (title Undo; OK, Cancel)
  comes first; after OK the editor has the text from before the reload
  and is modified. Redo asks "Redo Reload from Disk?".
- Step 4: the File Cache Conflict dialog: "Changes have been made to
  '<path>' in memory and on disk." with Keep Memory Changes and Load File
  System Changes (the default). Nothing writes the file while it is open.
- Step 5: Load File System Changes shows the file's text (one undo step
  that asks before it is undone); Keep Memory Changes (and Escape) keeps
  the editor's text, which the next save writes over the file.
- Step 6: the save does not write over the newer file; the File Cache
  Conflict dialog asks instead (the IDE looks at a file before every
  save).
- Step 7: the tab closes.
- Step 8: the tab stays open, its title in the error colour, and is not
  saved automatically. Closing it asks "Close Document: The document has
  been modified. Do you want to save the changes?" with Save (writes the
  file again), Discard and Cancel. Save All writes the file again too.
- Step 9: the tab of the old name closes (a deletion and a creation); the
  project tree shows the new name.
- Step 10: the project tree, Go to File and the language services follow
  the files created, changed and removed; open files take the changes as
  in steps 1 to 8.
- Step 11: every modified file is saved, then the open files are looked
  at and the project tree and the file index are made again.
- Step 12: the file is saved on quit.
- Differences from the reference: an editor with changes whose file was
  deleted stays open (the reference closes it; Pyoxide has no Local
  History yet to get the changes back); the conflict dialog has no Show
  Difference button (no diff viewer yet).

## Commands
`workbench.synchronize`, `workbench.save-all`, `editor.save`,
`editor.undo`, `editor.redo`, `dialog.choose {"button": ...}`,
`dialog.confirm`, `dialog.cancel`, `tabs.close`, `workbench.state`
(`dialog`, `tabs[].modified`, `tabs[].deleted`).

## Source
https://www.jetbrains.com/help/pycharm/system-settings.html (Synchronize
external changes), https://www.jetbrains.com/help/pycharm/file-cache-conflict.html,
IntelliJ Platform Community sources (`FileDocumentManagerImpl`,
`MemoryDiskConflictResolver`, `SaveAndSyncHandlerImpl`,
`FileEditorManagerImpl`, `UndoRedo`, LightEdit's `LightEditTabs`).
