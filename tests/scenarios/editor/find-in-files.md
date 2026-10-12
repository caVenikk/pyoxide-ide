# Find in files

The reference IDE's Find in Files popup, with its default keymaps: macOS
("Mac") and Windows/Linux ("Win"). Keystrokes come from the IntelliJ
Platform Community keymap files; option keys are the mnemonics of their
labels (see NOTICE). Covered by
`crates/platform/workbench/tests/find_in_files.rs` (keystrokes through both
keymaps), `crates/platform/workbench/tests/keymap.rs` (every binding) and
`crates/platform/project-search` (what is searched, masks, limits).

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Find in Files | Cmd+Shift+F | Ctrl+Shift+F | anywhere in the main window |
| Open the selected line | Enter | Enter | popup fields |
| Close the popup | Escape | Escape | popup fields |
| Next / previous line of the results | Down / Up | Down / Up | search field holding one line |
| Next / previous line of the results | Cmd+G / Cmd+Shift+G | F3, Ctrl+L / Shift+F3, Ctrl+Shift+L | popup fields |
| A page down / up in the results | Page Down / Page Up | Page Down / Page Up | search field |
| New line in the search field | Cmd+Shift+Enter | Ctrl+Shift+Enter | search field |
| Match Case / Words / Regex | Ctrl+Option+C / W / X, Option+C / W / X | Alt+C / W / X | popup fields |
| File mask on or off | Ctrl+Option+K, Option+K | Alt+K | popup fields |

## Steps and expected behaviour

### Opening the popup
1. Select `needle` in an editor and press Find in Files.
   - A popup opens over the window, centred, 115 px below the top of the
     window. Its header reads "Find in Files", then the counter, then the
     File mask check box and field and the pin button. Below are the
     search field with `needle` selected and the focus in it, the scope
     ("In Project") and the matching lines.
2. Press Find in Files with nothing selected.
   - The field holds the search of the editor's open find bar, else the
     popup's last search, else the last search of the session.

### What is searched
3. Search a project with `.git`, `__pycache__`, `*.pyc`, a virtual
   environment, a `.gitignore` and a `.idea` folder.
   - Every file of the project is searched except: the hidden names
     ("ignored files and folders": `.git`, `__pycache__`, `*.pyc`, ...),
     excluded folders (unless the folder searched is itself excluded),
     binary files and files over 20 MB. Ignore files such as `.gitignore`
     do not leave files out by themselves; which folders count as excluded
     is the project's setting.
   - The reference IDE's project files (`.idea/`, `*.iml`, `*.ipr`,
     `*.iws`) are searched, but their matches are held back unless the
     mask names them or the folder searched is inside `.idea`.
   - An open file with unsaved changes is searched as it is in the editor.

### Results
4. Type a query.
   - The search starts 100 ms after the last change; the rows of the
     previous search stay until the first new row arrives. Rows come in
     while the search runs: one row per line with matches, the line's text
     without its indentation in the editor font with the matches marked,
     the file name and line number on the right. The first file found
     stays at the top, the others follow by path, lines by number.
   - The header counts "5 matches in 3 files" ("1 match in 1 file").
5. Search for something with more than 100 matches.
   - The search stops at 100 matches and the counter reads
     "100+ matches in 12+ files".
6. Use Down and Up, or Find Next and Find Previous.
   - The selection moves through the rows (not past the ends) and the
     list scrolls to show it; the focus stays in the field. While the
     search field holds several lines Up and Down move its caret.
7. Press Enter or double-click a row.
   - The file opens with the caret at the start of the match, its line in
     the middle of the view, and the popup closes.

### Options and the file mask
8. Switch Match Case, Words or Regex.
   - The search runs again with the option. A pattern that does not
     compile says "Bad pattern ..." in place of the rows; one that matches
     the empty string says "Regular expression matches empty string".
9. Check File mask (Option+K / Alt+K) and type `*.txt, !app*`.
   - The focus moves to the mask, its text selected. Only files whose
     names match are searched: masks separated by commas, `*` and `?`
     wildcards, `!` to leave names out, case ignored. Unchecking it puts
     the focus back in the search field.
   - The mask and its check box are remembered across sessions: a mask
     left on is on the next time, and the last masks are offered.

### Closing and pinning
10. Click elsewhere in the window.
    - The popup closes, unless it is pinned. Switching to another
      application leaves it open.
11. Pin the popup and open a result.
    - The popup stays open behind the editor; the pin is remembered.
12. Press Escape.
    - The popup closes, also when pinned, and the focus goes back to where
      it was.

## Notes
- The reference also offers the Module scope; a Pyoxide project has one
  content root, so Module is left out.
