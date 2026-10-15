# File operations in the project tool window

## Steps
1. Open a folder with `app.py`, `notes.txt`, `src/m.py`, `src/pkg/n.py`
   and `docs/index.md`. Select `src/m.py` in the project tool window and
   press Cmd+N (macOS) or Alt+Insert (Windows, Linux).
2. Choose *File*, type `m.py` and press Enter; then type `pkg/sub/new.py`
   and press Enter.
3. Select `app.py` and choose *New > Directory* from its context menu.
   Type `src`, then `a.b`, then press Enter.
4. Open `app.py`, type a character without saving, select `app.py` in
   the tree and press Shift+F6. Type `main`, press Enter.
5. Select `src/pkg` (expanded, with `n.py` open in a tab) and rename it to
   `lib`. Mark `docs` as excluded and rename it to `manual`.
6. Select `src` (with `src/m.py` open) and press Delete (macOS also
   Backspace or Cmd+Backspace). Press Escape; press Delete again and
   Enter.
7. Right-click a file, a folder and the project root.
8. In the Python product, select `src` and choose *New > Python File*.
   Press Down, type `test_m` and press Enter. Then choose *New > Python
   Package*, type `pkg`, then `app.core`, and press Enter.
9. Select `app.py`, press Cmd+C (Ctrl+C), select `src` and press Cmd+V
   (Ctrl+V). Type `copy` and press Enter.
10. Open `src/m.py`, select it, press Cmd+X (Ctrl+X), select `docs` and
    press Cmd+V (Ctrl+V), then Enter.
11. Select `src` and press F6; type the path of `src/pkg` into *To
    directory* and press Enter. Press Escape.
12. Copy `notes.txt` (F5) into a folder that has a `notes.txt`.
13. Select `src/pkg/n.py` and press Cmd+Shift+C (Ctrl+Shift+C); open the
    context menu's *Copy Path/Reference…*.
14. Drag `notes.txt` onto `src/pkg`; drag `app.py` onto `src/m.py` with
    Option (macOS) or Ctrl held; drag `app.py` onto the project root.
15. Open `src/pkg/n.py`, then `src/m.py`, then `app.py`; rename `src/pkg`
    to `lib`, delete `src/m.py` and press Back (Cmd+[ / Ctrl+Alt+Left).
16. With `src` expanded and `src/m.py` open, change `src/m.py` and add a
    file to `src` outside the IDE (no file watcher); right-click `src` and
    choose *Reload from Disk*.
17. Drag a row slowly over a folder row, a file row and onto itself.

## Expected
- Step 1: a popup titled *New* below the selected row lists *File* and
  *Directory*.
- Step 2: a popup titled *New File* with an empty field (placeholder
  *Name*) in the middle of the window. `m.py` shows the error "A file with
  the name 'm.py' already exists" above the field and the popup stays;
  the error goes away as soon as the name changes. `pkg/sub/new.py`
  creates the folder `sub` and the empty file; the popup closes, the new
  file is selected in the tree and opens in the editor, which gets the
  focus. Go to File finds it.
- Other names: an existing folder ("A directory with the name 'pkg'
  already exists"), `..` ("'..' is an invalid file name"), an ignored name
  such as `x.pyc` ("Trying to create a directory with an ignored name
  ('x.pyc'); the result will not be visible"). Pyoxide never creates
  outside the project: `../../x.py` and `~/x.py` are "outside the
  project" (the reference creates them).
- Step 3: *New Directory* checks the name while it is typed: `src` shows
  "A directory with the name 'src' already exists" as an error, `a.b` the
  warning 'Note: "." in the name is treated as a regular character. Use
  "/" instead if you mean to create nested directories'. Enter creates
  `a.b` in the project root (the folder of the selected file), selects it
  and keeps the focus in the tree. Folders are listed before files.
- Step 4: a dialog titled *Rename* asks "Rename file 'app.py' to:" with
  `app.py` in the field and `app` selected. A name with `/` disables
  *Refactor* and shows "'a/b' is not a valid identifier"; an existing name
  shows "File 'notes.txt' already exists.". After Enter the file is
  `main.py` on disk and in the tree (selected), the tab shows `main.py`
  and keeps the unsaved change, which is written to `main.py`.
- Step 5: "Rename directory 'pkg' to:"; the folder stays expanded under
  its new name and the tab of `n.py` shows `src/lib/n.py`. The excluded
  folder stays excluded under its new name (`.oxide/project.toml` lists
  `manual`).
- Step 6: a dialog titled *Delete* with a question icon asks 'Delete
  directory "src"?' and warns 'All files and subdirectories in "src" will
  be deleted. You might not be able to fully undo this operation!', with
  *Cancel* and *Delete* (the default). Escape keeps everything. Enter
  moves `src` to the system trash, removes its rows, selects the row
  that took its place and closes the tabs of its files without saving
  them. A file asks 'Delete file "notes.txt"?'.
- Step 7: the context menu is *New* (*File*, *Directory*, the product's
  rows); *Cut*, *Copy*, *Copy Path/Reference…*, *Paste* (only after Cut
  or Copy); *Find in Files…*, *Replace in Files…*; *Rename…*, *Refactor*
  (*Rename…*, *Move…*, *Copy…*); *Delete…*; *Mark Directory As* (*Mark
  File As*), separated by lines. On the project root *Cut*, *Copy*,
  *Rename…*, *Refactor* and *Delete…* cannot be chosen.
- Step 8: *New* lists *File*, *Directory*, *Python Package*, *Python
  File*. *Python File* opens the popup *New Python File* with the kinds
  *Python file*, *Python unit test* and *Python stub* under the field; Up
  and Down choose one. `test_m` becomes `src/test_m.py` with the
  reference's unit test template and opens in the editor. *New Python
  Package* checks the dotted path as it is typed (`pkg` already exists);
  `app.core` creates `src/app/__init__.py` and `src/app/core/__init__.py`
  (not one in `src`) and selects `src/app/core`.
- Step 9: Paste opens the dialog *Copy* with "Copy file …/app.py" in bold,
  *New name:* `app.py` with `app` selected, *To directory:* the path of
  `src` and *Open copy in editor* on; OK and Cancel. Enter copies to
  `src/copy.py`, selects it and opens it in the editor. The copied file
  stays in the clipboard for another Paste.
- Step 10: the cut row is greyed. Paste opens the dialog *Move* with "Move
  file …/m.py", *To directory:* the path of `docs` and *Open in editor*
  off; Refactor and Cancel. The file moves to `docs/m.py`, its tab
  follows, it is selected and the cut ends.
- Step 11: the dialog shows "Cannot place directory into its
  subdirectory." and stays open; Escape closes it.
- Step 12: a question titled *Copy Files/Directories* ("File 'notes.txt'
  already exists in directory '…'") offers *Skip* and *Overwrite* (the
  default); with several conflicts also *Skip for all* and *Overwrite
  for all*. Escape skips. A folder of the same name is an error
  ("Directory 'x' already exists.").
- Step 13: the clipboard holds the absolute path. *Copy
  Path/Reference…* offers *Absolute Path*, *File Name* and *Path from
  Content Root* (`src/pkg/n.py`).
- Step 14: dropping onto a folder opens the Move dialog with that folder;
  with the copy modifier the Copy dialog, onto a file its folder. Dropping
  where the file is already does nothing.
- Step 15: the navigation history follows: Back opens `src/lib/n.py`; the
  places in the deleted file are gone.
- Step 16: *Reload from Disk* (with the refresh icon) sits after *Delete…*
  in its own group, before *Mark Directory As*, for files, folders and the
  project root. It writes nothing first: the open files under the
  selection are reloaded when they changed outside, the folders are
  listed again (the new file shows), and the status bar says "File
  synchronization completed successfully.". Files elsewhere are left
  alone.
- Step 17: the dragged row's icon and name follow the pointer where they
  were in the row, in the normal text colour on the tree's background at
  70 % (no box of their own). The row under the pointer gets the drop
  target rectangle across the tree: the drag and drop row colour with a
  1 px border in the drag and drop border colour (blue in both themes). A
  folder over itself or its own content gets none.
- With the file watcher on, the tree's own renames, moves, copies and
  deletes are not external changes: no tab closes, nothing reloads, no
  File Cache Conflict asks, and a renamed or moved open file is watched
  under its new path (a change made outside reloads it).
- File system errors open a dialog with the reference's title: *Cannot
  Create File* (a file), *Error* (a folder, a delete, a copy or move),
  *Rename*, *Create Package Error*. Name errors stay in the popup.
- The language services hear about every created, deleted and renamed
  file (`workspace/didChangeWatchedFiles`); a renamed open file is closed
  under its old name and opened under the new one.

## Commands
`project.tree.new`, `project.tree.new-file {"name"?, "directory"?}`,
`project.tree.new-directory {"name"?, "directory"?}`,
`project.tree.new-item {"item", "kind"?, "name"?, "directory"?}`,
`project.tree.rename {"path"?, "name"?}`, `project.tree.delete
{"paths"?}`, `project.tree.cut {"paths"?}`, `project.tree.copy
{"paths"?}`, `project.tree.paste {"directory"?}`,
`project.tree.copy-element {"paths"?, "to"?, "name"?}`,
`project.tree.move {"paths"?, "to"?}`, `project.tree.reload-from-disk
{"paths"?}`, `project.tree.copy-paths
{"paths"?}`, `project.tree.copy-path {"kind", "paths"?}`,
`name-input.confirm`, `name-input.cancel`, `name-input.next-kind`,
`name-input.previous-kind`, `name-input.next-field`,
`name-input.previous-field`, `name-input.toggle-option`,
`dialog.confirm`, `dialog.cancel`, `dialog.choose {"button": "Delete"}`,
`workbench.back`, `workbench.state` (`tree.name_popup`,
`tree.transfer`, `tree.clipboard`, `tree.drop_target`, `dialog`,
`tree.rows`,
`tree.selected`, `tabs`, `navigation`).

## Source
IntelliJ Platform Community at `pycharm/2026.2.3` (Apache-2.0):
`NewElementAction`, `CreateFileAction`, `FileNameState.kt`,
`CreateDirectoryOrPackageAction`, `CreateDirectoryOrPackageHandler`,
`NewItemPopupUtil`, `NewItemSimplePopupPanel`,
`IdeViewForProjectViewPane`, `RenameElementAction`, `RenameDialog`,
`RenamePsiFileProcessor`, `DeleteHandler`, `DeleteUtil`, `TrashBin`,
`CopyPasteDelegator`, `CopyHandler`, `CopyFilesOrDirectoriesHandler`,
`CopyFilesOrDirectoriesDialog`, `MoveHandler`,
`MoveFilesOrDirectoriesHandler`, `MoveFilesOrDirectoriesUtil`,
`MoveFilesOrDirectoriesDialog`, `SkipOverwriteChoice`, `CopyPathsAction`,
`CopyReferenceAction`, `ProjectViewDropTarget`, Python's
`CreatePythonFileAction`, `CreatePackageAction` and file templates,
`GeneralSettings` (`deleteToBin`), `LangActions.xml`
(`ProjectViewPopupMenu`), the keymaps `$default.xml` and
`Mac OS X 10.5+.xml`, and the messages in `IdeBundle`, `CoreBundle`,
`LangBundle` and `RefactoringBundle`;
https://www.jetbrains.com/help/pycharm/creating-and-managing-files.html.
