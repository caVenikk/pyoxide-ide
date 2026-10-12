# Search Everywhere and Go to File

The reference IDE's current ("split") Search Everywhere popup with the
All, Files, Actions and Text tabs (Classes and Symbols come with a
language server), with its default keymaps: macOS ("Mac") and
Windows/Linux ("Win"). Keystrokes come from the IntelliJ Platform
Community keymap files (see NOTICE). Covered by
`crates/platform/workbench/tests/search_everywhere.rs` (keystrokes,
modifier events, clicks and scroll events through both keymaps),
`crates/platform/workbench/tests/keymap.rs` (every binding),
`crates/platform/name-match` (matching and degrees, cases of the
reference's matcher tests) and `crates/platform/file-index` (the index,
Go to File's order with the reference's own Go to File cases,
`name:line:column`, absolute paths). Recent Files has its own scenario,
`recent-files.md`.

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Search Everywhere (All) | Shift twice | Shift twice | anywhere in the main window |
| Go to File (Files) | Cmd+Shift+O | Ctrl+Shift+N | anywhere in the main window |
| Go to Action (Actions) | Cmd+Shift+A | Ctrl+Shift+A | anywhere in the main window |
| Find Text (Text) | Cmd+Option+Shift+E | Ctrl+Alt+Shift+E | anywhere in the main window |
| Next / previous tab | Tab / Shift+Tab, Cmd+Shift+] / [, Ctrl+Right / Left | Tab / Shift+Tab, Alt+Right / Left | popup |
| Next / previous result | Down / Up | Down / Up | popup |
| A page down / the first result | Page Down, Cmd+Down / Page Up, Cmd+Up | Page Down, Ctrl+Down / Page Up, Ctrl+Up | popup |
| Open or run the result | Enter, Shift+Enter | Enter, Shift+Enter | popup |
| Previous / next search of the tab | Option+Up / Option+Down | Alt+Up / Alt+Down | popup |
| Widen the scope (non-project items, All Places, disabled actions) | Ctrl+Option+N, Option+N | Alt+N | popup |
| Match Case / Words / Regex | Ctrl+Option+C, Option+C / Ctrl+Option+W, Option+W / Ctrl+Option+X, Option+X | Alt+C / Alt+W / Alt+X | popup, Text tab |
| Close | Escape | Escape | popup |

## Steps and expected behaviour

### Opening
1. Press Shift twice quickly.
   - The popup opens a quarter down the window, centred, on the All tab:
     the tabs (All, Files, Actions, Text) with the tab's scope control on
     the right, and the search field with the focus and a history button
     at its left. With nothing typed the recently opened files are listed,
     newest first, without the file of the active editor; with no recent
     files only the tabs and the field show.
   - The taps count when each step (down, up, down, up) follows the last
     within 300 ms, no other key or modifier comes between, and the first
     comes at least 100 ms after a typed key (Shift for a capital is not a
     tap). The popup opens on the second release.
2. Press Go to File, Go to Action or Find Text.
   - The popup opens on Files, Actions or Text. The field holds the text
     selected in the editor (one line; not for Go to Action), else the
     tab's last search, selected.
3. Press the shortcut of the tab shown again.
   - The tab's scope widens: "Include non-project items" on All, All Places
     on Files and Text, "Include disabled actions" on Actions; again, it
     narrows. The shortcut of another tab switches to it; the text stays.
     The scope chosen on Files and Text is kept for the next time.

### Files
4. Type a name: `util`, `NaUT`, `nut`, `na ut`, `*util*`.
   - 100 ms after the last key the files and folders whose names match are
     listed, best first: a file named exactly as typed, then names starting
     as typed (in their case), then by the matching degree (word starts,
     capitals typed, fewer gaps, the end of the name); files of one name
     (with any extension) open in editors first, then those in the current
     file's folder, the shallower first; folders last. The file icon, the
     name and the folder in grey; only the selected row shows the matched
     letters.
5. Type a folder part: `flask/app`, `src/f/app.py`, `flaskapp`, `tests/`.
   - Only files in folders that match the part before the last `/` are
     listed, by how well their folder path matches it; without a slash the
     first letters may stand for a folder; a trailing `/` lists folders
     only.
6. Type an absolute path: of a project file, of an excluded one, of a file
   or folder outside the project, `~/` and a path in the home folder (not
   on Windows).
   - The file or folder comes first, whatever the scope. A path that does
     not exist, and `/` alone, find nothing by path.
7. Type `app.py:12`, `app.py:12:5`, `app.py(12)`, `app.py 12`.
   - The file is found by its name; Enter opens it at line 12 (column 5).
8. Enter on a folder.
   - The project tool window shows and selects it.
9. The scope list (Project Files / All Places) and the filter button.
   - All Places adds the content of excluded folders. The filter lists
     the file types (and Directory) with All, None and Invert; an unchecked
     type is left out of the list and stays unchecked across sessions.
10. Type something only in an excluded folder (`site` in `.venv`).
    - Nothing in the project: the scope widens by itself and the file shows
      with the excluded background; a text that no longer contains `site`
      narrows it again. A scope the person chose stays.
11. Type something that is nowhere.
    - "Set scope to all places or use Find in Files (shortcut) to perform
      full-text search."; the links widen the scope or open Find in Files
      with the text.

### Text
12. On the Text tab type `needle`.
    - The lines with matches, each with the line (without its
      indentation) and the file name and line number on the right, in Find
      in Files' order of files. Enter opens the file at the match.
13. Match Case, Words, Regex and the file mask.
    - They search again at once; they are the same options and mask as
      Find in Files', both ways. With nothing found the options used are
      listed with "Clear all options".

### All and Actions
14. Type on the All tab.
    - Recent files that match come first, then within each kind the exact
      matches, then everything by weight; text lines last. The filter
      button lists the kinds (Recent Files, Files, Actions, Text) to leave
      out.
15. On Actions type `save all`.
    - The commands whose titles match (each word starting a word of the
      title), with their shortcuts after the title; unavailable ones only
      with "Include disabled actions". Enter closes the popup and runs the
      command where the focus was.

### Lists and history
16. More results than fit (50 of each kind at first).
    - A spinner row ends the list while there are more; scrolling near the
      end, or Down onto it, loads the next ones below the rows shown.
17. Up on the first row, Down on the last.
    - The selection goes round to the other end once every row is loaded;
      while more rows can come, Up on the first row stays.
18. Page Down / Page Up.
    - Page Down goes down by the rows shown less four; Page Up goes to the
      first row.
19. Option+Up / Alt+Up, or the history button.
    - The tab's previous searches, newest first (on All the searches of all
      tabs); a search is kept when the popup closes.
20. Escape, or a click elsewhere in the window.
    - The popup closes (Escape gives the focus back). Its place and size
      are kept across sessions; reopened with the same text, the row
      selected before is selected again.

## Notes
- The reference also has the Classes and Symbols tabs (they come with a
  language server), a preview, "Open in Find Tool Window", typo-tolerant
  action search and a "Recent Locations" row; they are not here yet.
- Fuzzy file search and machine-learning reordering are off by default in
  the reference and are left out.
- A full name includes the project folder's name, as in the reference: in
  a project folder named `flask`, `flask/app` matches the root itself, so
  short paths can rank first.

## Source
Observed behaviour of the reference IDE and the IntelliJ Platform
Community sources (see NOTICE).
