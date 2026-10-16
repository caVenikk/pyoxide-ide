# Find in files

The reference IDE's Find in Files and Replace in Files popup, with its
default keymaps: macOS ("Mac") and Windows/Linux ("Win"). Keystrokes come
from the IntelliJ Platform Community keymap files; option keys are the
mnemonics of their labels (see NOTICE). Covered by
`crates/platform/workbench/tests/find_in_files.rs` (keystrokes through both
keymaps, synthetic mouse and scroll events),
`crates/platform/workbench/tests/keymap.rs` (every binding) and
`crates/platform/project-search` (what is searched, masks, limits,
replacing in files).

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Find in Files | Cmd+Shift+F | Ctrl+Shift+F | anywhere in the main window |
| Replace in Files | Cmd+Shift+R | Ctrl+Shift+R | anywhere in the main window |
| Open the selected line | Enter, Shift+Enter | Enter, Shift+Enter | popup fields (Find) |
| Replace the selected match | Enter, Shift+Enter | Enter, Shift+Enter | popup fields (Replace) |
| Replace All | Shift+Option+Enter | Shift+Alt+Enter | popup fields (Replace) |
| Close the popup | Escape | Escape | popup fields |
| Next / previous line of the results | Down / Up | Down / Up | search and replace fields holding one line |
| Next / previous line of the results | Cmd+G / Cmd+Shift+G | F3, Ctrl+L / Shift+F3, Ctrl+Shift+L | popup fields |
| Find Next / Find Previous in the file | Cmd+G / Cmd+Shift+G | F3, Ctrl+L / Shift+F3, Ctrl+Shift+L | preview (the editor's keys) |
| Edit Source | Cmd+Down, F4 | F4 | popup fields (the selected line), preview (at its caret) |
| A page down / up in the results | Page Down / Page Up | Page Down / Page Up | search field |
| Next / previous field | Tab / Shift+Tab | Tab / Shift+Tab | popup fields |
| History of the field (masks and folders for theirs) | Option+Down | Alt+Down | popup fields |
| New line | Cmd+Shift+Enter | Ctrl+Shift+Enter | search and replace fields |
| Match Case / Words / Regex | Ctrl+Option+C / W / X, Option+C / W / X | Alt+C / W / X | popup fields |
| File mask on or off | Ctrl+Option+K, Option+K | Alt+K | popup fields |
| In Project / Module / Directory / Scope | Ctrl+Option+P / M / D / S, Option+P / M / D / S | Alt+P / M / D / S | popup fields, preview |
| Recursively (Directory) | Ctrl+Option+Y | Alt+Y | popup fields, preview |
| Break the line, indent | Enter, Tab | Enter, Tab | preview (the editor's keys) |
| Choose the folder | Shift+Enter | Shift+Enter | folder field |
| Item of an open list | Up, Down, Enter, Escape | Up, Down, Enter, Escape | the list |

## Steps and expected behaviour

### Opening the popup
1. Select `needle` in an editor and press Find in Files.
   - A popup opens over the window, centred, 115 px below the top of the
     window. Its header reads "Find in Files", then the counter, then the
     File mask check box and field and the pin button. Below are the
     search field with `needle` selected and the focus in it (an empty
     field with the focus shows only the caret; without it, "Search" and
     "Replace" in grey), the scope
     tabs ("In Project", "Module", "Directory", "Scope", each with its
     mnemonic letter underlined), the matching lines and, under a
     splitter, the preview.
   - The first time, the file mask is on with `*.py`; later the popup
     opens as it was left.
2. Press Find in Files with nothing selected.
   - The field holds the search of the editor's open find bar, else the
     popup's last search, else the last search of the session.
   - With an empty field there are no results, also with the file mask on:
     the list and the preview are empty, "Type search query to find in
     files" is centred in their place (in Replace in Files too) and the
     header has no counter.
3. Select a folder in the project tool window and press Find in Files.
   - The Directory scope is selected with that folder (a selected file
     gives its folder).
   - The tree's context menu offers *Find in Files…* and *Replace in
     Files…* above *Mark Directory As*, for folders and files alike; they
     open the popup the same way.

### What is searched
4. Search a project with `.git`, `__pycache__`, `*.pyc`, a virtual
   environment, a `.gitignore` and a `.idea` folder.
   - Every file of the project is searched except: the hidden names
     ("ignored files and folders": `.git`, `__pycache__`, `*.pyc`, ...),
     excluded folders (unless the folder searched is itself excluded),
     binary files and files over 20 MB. Ignore files such as `.gitignore`
     do not leave files out by themselves; which folders count as excluded
     is the project's setting, and a folder marked Excluded in the tree is
     left out of the next search at once.
   - The reference IDE's project files (`.idea/`, `*.iml`, `*.ipr`,
     `*.iws`) are searched, but their matches are held back unless the
     mask names them or the folder searched is inside `.idea`.
   - An open file with unsaved changes is searched as it is in the editor.

### Results
5. Type a query.
   - The search starts 100 ms after the last change, with the loading
     indicator next to the counter; the rows of the previous search stay
     until the first new row arrives. Rows come in while the search runs:
     one row per line with matches, the line's text without its
     indentation in the editor font with the matches marked, the file name
     and line number on the right. The first file found stays at the top,
     the others follow by path, lines by number.
   - The header counts "5 matches in 3 files" ("1 match in 1 file").
6. Search for something with more than 100 matches.
   - The search stops at 100 matches and the counter reads
     "100+ matches in 12+ files".
   - Scrolling to the end of the list (or a list that shows every row)
     loads 50 more, with a loading indicator at the bottom; the rows on
     screen stay where they are.
7. Use Down and Up, or Find Next and Find Previous.
   - The selection moves through the rows (not past the ends) and the
     list scrolls to show it; the focus stays in the field. While the
     search field holds several lines Up and Down move its caret, and the
     bottom row reads "Use ⌘G and ⇧⌘G to select usages".
8. Press Enter or double-click a row.
   - The file opens with the caret at the start of the match, its line in
     the middle of the view, and the popup closes. Edit Source (Cmd+Down
     or F4 / F4) does the same.
### Preview
9. Select a row.
    - 50 ms later the preview shows its file in an editor with its syntax
      colours and line numbers; the row's matches are marked (outlined in
      Replace in Files), the caret is on the first one and its line is in
      the middle. The title reads the file name and its folder in the
      project.
10. Click into the preview and edit.
    - It is a full editor on the file: typing, selection, the clipboard,
      undo and redo and every editor key work, and the marked matches move
      with the text. Enter breaks the line and Tab indents (they do not
      open a result or move to another field); Escape closes the popup;
      the option, scope and mask keys still switch the popup's options.
    - Find Next and Find Previous (Cmd+G / F3) go to the next match of the
      last search in a file, as in any editor, with the "not found, press
      ... to search from the top" hint at the end; the selected row stays.
    - Edit Source (Cmd+Down or F4 / F4) opens the file in an editor at the
      preview's caret.
    - A file open in a tab shows every edit at once, both ways, and the tab
      saves it. A file that is not open is saved like any edit (after the
      idle time, when the window loses the focus, when the preview shows
      another file and when the popup closes), without opening a tab;
      opening it from the popup continues with the edited text.
    - The rows stay as they were found until the next search, as in the
      reference, but where their matches are follows the edits: another
      row of the file is marked where its match is now, a row opens there,
      and Replace changes the match there.
    - Moving the mouse over the preview fades in the Soft-Wrap button
      ("Soft-Wrap Preview Editor"), a plain icon 46 pixels below the
      text's top and 49 left of its scrollbar (half a second);
      it stays 1.5 s after the mouse stopped (longer while the mouse is
      over it) and fades out (a second). A click turns soft wraps on or off
      in both previews (Find in Files and Search Everywhere); the choice is
      remembered across sessions. The focus stays where it was.
11. Drag the splitter between the list and the preview.
    - The share of each changes and is remembered across sessions
      (default: a third for the list).

### Options, the file mask and the scope
12. Switch Match Case, Words or Regex.
    - The search runs again with the option. A pattern that does not
      compile says "Bad pattern ..." in place of the rows; one that matches
      the empty string says "Regular expression matches empty string".
13. Check File mask (Option+K / Alt+K) and type `*.txt, !app*`.
    - The label's mnemonic letter is underlined while Option (Alt) is
      held. The focus moves to the mask, its text selected. Only files whose
      names match are searched: masks separated by commas, `*` and `?`
      wildcards, `!` to leave names out, case ignored. Unchecking it puts
      the focus back in the search field; unchecked, the mask cannot be
      edited.
    - The arrow (or Option+Down / Alt+Down) lists the masks used, newest
      first. The mask and its check box are remembered across sessions.
14. Choose Directory (Option+D / Alt+D).
    - The folder field gets the focus; the arrow lists the folders used,
      "…" (or Shift+Enter) shows a folder chooser, the tree button
      (Ctrl+Option+Y / Alt+Y) searches the subfolders or not. A folder that is
      not there says "Directory is not found".
15. Choose Module (Option+M / Alt+M) or Scope (Option+S / Alt+S).
    - Module offers the project's one module, the project, in a list as
      wide as the scope list; it searches the whole project.
    - The scope list offers Project Files, Recently Viewed Files,
      Recently Changed Files, Open Files and Current File (those without
      files are left out); the choice is remembered.

### Histories
16. Press Option+Down / Alt+Down in the search field (or click its
    history icon).
    - The recent searches (shared with the find bar), newest first, show
      below the field; Enter or a click puts one in the field. The replace
      field has the replacements.

### Replace in Files
17. Press Replace in Files and type a replacement.
    - The title reads "Replace in Files"; every match is a row of its own.
18. Press Enter.
    - The selected match is replaced and its row goes; the next row is
      selected and the popup stays open. An open file changes in its
      editor (one undo step); a file that is not open changes on disk,
      keeping its encoding and line separator.
19. Press Replace All (Shift+Option+Enter / Shift+Alt+Enter, or the
    button).
    - With matches in several files a confirmation asks "Replace 7
      occurrences of 'needle' across 5 files with 'pin'?" (counts with `+`
      while there may be more). Every match of the scope is replaced, not
      only the rows shown; each open file changes as one undo step. The
      popup closes and the status bar reads "6 occurrences replaced".
    - When a file cannot be written nothing is replaced and the status
      bar names the read-only files.

### Closing, pinning, moving
20. Click elsewhere in the window.
    - The popup closes, unless it is pinned. Switching to another
      application leaves it open.
21. Pin the popup and open a result.
    - The popup stays open behind the editor; the pin is remembered.
22. Drag the header, or an edge or corner.
    - The popup moves or resizes; its place and size are remembered across
      sessions.
23. Press Escape.
    - The popup closes, also when pinned, and the focus goes back to where
      it was.

## Notes
- A Pyoxide project is one module, so the Module tab lists the project.
- The reference undoes a Replace All over several files as one global
  step ("Undo Replace?"); Pyoxide undoes it per open file, and a file
  changed on disk is not undone from the editor.
