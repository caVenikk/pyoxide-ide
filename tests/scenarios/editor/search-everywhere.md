# Search Everywhere and Go to File

The reference IDE's Search Everywhere popup with the All, Files and
Actions tabs (Classes and Symbols come with a language server, Text with
the next stage), with its default keymaps: macOS ("Mac") and Windows/Linux
("Win"). Keystrokes come from the IntelliJ Platform Community keymap files
(see NOTICE). Covered by `crates/platform/workbench/tests/search_everywhere.rs`
(keystrokes and modifier events through both keymaps, synthetic scroll
events), `crates/platform/workbench/tests/keymap.rs` (every binding),
`crates/platform/name-match` (matching and degrees, cases of the
reference's matcher tests) and `crates/platform/file-index` (the index,
Go to File's order, `name:line:column`).

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Search Everywhere (All) | Shift twice | Shift twice | anywhere in the main window |
| Go to File (Files) | Cmd+Shift+O | Ctrl+Shift+N | anywhere in the main window |
| Go to Action (Actions) | Cmd+Shift+A | Ctrl+Shift+A | anywhere in the main window |
| Next / previous tab | Tab / Shift+Tab, Cmd+Shift+] / [, Ctrl+Right / Left | Tab / Shift+Tab, Alt+Right / Left | popup |
| Next / previous result | Down / Up | Down / Up | popup |
| A page down / the first result | Page Down, Cmd+Down / Page Up, Cmd+Up | Page Down, Ctrl+Down / Page Up, Ctrl+Up | popup |
| Open or run the result | Enter, Shift+Enter | Enter, Shift+Enter | popup |
| Previous / next search of the tab | Option+Up / Option+Down | Alt+Up / Alt+Down | popup |
| Include non-project items (Actions: disabled actions) | Ctrl+Option+N, Option+N | Alt+N | popup |
| Close | Escape | Escape | popup |

## Steps and expected behaviour

### Opening
1. Press Shift twice quickly.
   - The popup opens a quarter down the window, centred, on the All tab:
     the tabs (All, Files, Actions) with "Include non-project items" on
     the right, and the search field with the focus. With nothing typed
     the recently opened files are listed, newest first, without the file
     of the active editor; with no recent files only the tabs and the field
     show.
   - The taps count when each step (down, up, down, up) follows the last
     within 300 ms, no other key or modifier comes between, and the first
     comes at least 100 ms after a typed key (Shift for a capital is not a
     tap). The popup opens on the second release.
2. Press Go to File, or Go to Action.
   - The popup opens on Files or Actions. The field holds the text selected
     in the editor (one line; not for Go to Action), else the tab's last
     search, selected.
3. Press the shortcut of the tab shown again.
   - "Include non-project items" (on Actions "Include disabled actions")
     switches on, again off. The shortcut of another tab switches to it;
     the text stays.

### Files
4. Type a name: `util`, `NaUT`, `nut`, `na ut`, `*util*`.
   - 100 ms after the last key the files and folders whose names match are
     listed, best first: names starting as typed (in their case), then by
     the matching degree (word starts, capitals typed, fewer gaps, the end
     of the name); files of one name (with any extension) open in editors
     first, then those in the current file's folder, the shallower first;
     folders last. The file icon, the name and the folder in grey; the
     selected row shows the matched letters.
5. Type a folder part: `flask/app`, `src/f/app.py`, `flaskapp`.
   - Only files in folders that match the part before the last `/` are
     listed, by how well their folder path matches it; without a slash the
     first letters may stand for a folder. A path typed in full comes
     first.
6. Type `app.py:12`, `app.py:12:5`, `app.py(12)`, `app.py 12`.
   - The file is found by its name; Enter opens it at line 12 (column 5).
7. Enter on a folder.
   - The project tool window shows and selects it.
8. Type something only in an excluded folder (`site` in `.venv`).
   - Nothing in the project: non-project items are included at once and
     the file shows with the excluded background; another text that does
     not contain `site` goes back to project items.

### All and Actions
9. Type on the All tab.
   - Recent files that match come first, then files and actions by
     weight.
10. On Actions type `save all`.
    - The commands whose titles match (each word starting a word of the
      title), with their shortcuts after the title; unavailable ones only
      with "Include disabled actions". Enter closes the popup and runs the
      command where the focus was.

### Lists and history
11. More results than the tab shows (30, on All 15 of each kind).
    - A "… more" row ends the list; Enter on it loads the next ones below
      the rows shown and selects the first new one.
12. Up on the first row, Down on the last.
    - The selection goes round to the other end.
13. Option+Up / Alt+Up.
    - The tab's previous searches, newest first (on All the searches of all
      tabs); a search is kept when the popup closes.
14. Escape, or a click elsewhere in the window.
    - The popup closes (Escape gives the focus back). Its place and size
      are kept across sessions.

## Notes
- The reference also has the Classes, Symbols and Text tabs, a scope list
  and a file type filter on Files, a preview and "Open in Find Tool
  Window"; they come later (Text in the next stage, Classes and Symbols
  with a language server).
- The shipping reference (the "split" Search Everywhere) loads more rows
  as the list is scrolled instead of "… more" rows; this stage keeps the
  "… more" rows of the earlier version.
