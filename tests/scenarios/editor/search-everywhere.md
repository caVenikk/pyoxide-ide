# Search Everywhere and Go to File

The reference IDE's Search Everywhere popup as its current release shows
it (the popup with "… more" rows and a footer), with the All, Files,
Actions and Text tabs, and its default keymaps: macOS ("Mac") and
Windows/Linux ("Win"). The reference shows Classes and Symbols only when a
language provides them; they come with a language server. Keystrokes come
from the IntelliJ Platform Community keymap files (see NOTICE). Covered by
`crates/platform/workbench/tests/search_everywhere.rs` (keystrokes,
modifier events and clicks through both keymaps),
`crates/platform/workbench/tests/keymap.rs` (every binding),
`crates/platform/name-match` (matching and degrees, cases of the
reference's matcher tests) and `crates/platform/file-index` (the index,
the reference's own Go to File cases, the order on the Flask repository's
layout, `name:line:column`, absolute paths). Recent Files has its own
scenario, `recent-files.md`. The preview is the one of Find in Files
(`find-in-files.md`, step 10 for its Soft-Wrap button), read-only here.

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
| Open or run the result; "… more" | Enter, Shift+Enter | Enter, Shift+Enter | popup |
| Previous / next search of the tab | Option+Up / Option+Down | Alt+Up / Alt+Down | popup |
| Widen the scope (non-project items, All Places, disabled actions) | Ctrl+Option+N, Option+N; Ctrl+Option+P | Alt+N; Alt+P | popup |
| The scope list (Files, Text) | Ctrl+Option+O | Alt+O | popup |
| Match Case / Words / Regex | Ctrl+Option+C, Option+C / Ctrl+Option+W, Option+W / Ctrl+Option+X, Option+X | Alt+C / Alt+W / Alt+X | popup, Text tab |
| Close | Escape | Escape | popup |

## Steps and expected behaviour

### Opening
1. Press Shift twice quickly.
   - The popup opens a quarter down the window, centred, on the All tab:
     the tabs (All, Files, Actions, Text) with the tab's buttons on the
     right, and the search field with the focus and a history button at
     its left. With nothing typed the recently opened files are listed,
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
4. The buttons right of the tabs.
   - Files and Text: the scope list ("Project Files", its P underlined;
     Ctrl+Option+O / Alt+O opens it, Ctrl+Option+P / Alt+P toggles it).
     All and Actions: the checkbox. Then the Preview toggle (not on
     Actions), on All and Files the filter, and "Open Results in Find
     Window" (not available until the Find tool window).

### Files
5. Type `flask/app` in the Flask repository (project folder `flask`).
   - 100 ms after the last key: `app.py` (`flask`), `app.py`
     (`flask.sansio`), `app.py` (`tests.test_apps.cliapp`), `appcontext.rst`
     (`docs`), `appdispatch.rst` and `appfactories.rst`
     (`docs/patterns`), the folder `apps`, as the reference shows them;
     then `task_app`, `test_appctx.py`, `test_apps`,
     `typing_app_decorators.py`, `blueprintapp`, `cliapp`,
     `importerrorapp.py`, `multiapp.py`, and "… more".
   - The order: a path typed in full or a file named as typed (`app.py`
     for `app`) first; then names that start with the typed name, then
     names with it at a word start, then inside a word; each by the
     matching degree and the name, then open and recent files, files in
     the current file's folder, project before non-project items, the
     shallower, and by path. Folders rank as files do. A folder part
     (`flask/`, `src/f/`, or the first letters without a slash,
     `flaskapp`) only filters.
   - Each row: the icon (a folder with `__init__.py` shows the package
     icon), the name, and in grey its location: a Python file's package
     from its source root (`flask.sansio`), another file's folder
     (`docs/patterns`), a folder's own path. Only the selected row shows
     the matched letters, in the name and in the location.
   - The footer: the selected file's path from its source root
     (`flask/app.py`; elsewhere from the home folder) and "Open In Right
     Split" (it opens the file in the editor until editor splits come).
   - `src` is a source root when the folder opens, as the reference marks
     it (also the root package folder that `setup.py`'s `package_dir`
     names).
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
      with the excluded background; a text that still contains `site`
      keeps it, another narrows it again. A scope the person chose stays.
11. Type something that is nowhere.
    - "Set scope to all places or use Find in Files (shortcut) to perform
      full-text search."; the links widen the scope or open Find in Files
      with the text.

### Text
12. On the Text tab type `needle`.
    - The lines with matches, each with the text icon, the line (without
      its indentation) in the UI font with the syntax colours of its file
      and the file name and line number on the right, in Find in Files'
      order of files. The selected row shows its text in the selection's
      colour with the matches marked. Enter opens the file at the match.
13. Match Case, Words, Regex and the file mask.
    - They search again at once; they are the same options and mask as
      Find in Files', both ways. With nothing found the options used are
      listed with "Clear all options".

### All and Actions
14. Type on the All tab.
    - Recent files that match come first, then everything by weight
      (files in the Files tab's order); text lines last. The filter
      button lists the kinds (Recent Files, Files, Actions, Text) to leave
      out.
15. On Actions type `save all`.
    - The commands whose titles match (each word starting a word of the
      title), with their shortcuts after the title; unavailable ones only
      with "Include disabled actions". The footer shows the selected
      command's description. Enter closes the popup and runs the command
      where the focus was.

### Lists and history
16. More results than a page (15 of each kind on All and Files, 30 on
    Actions and Text).
    - A "… more" row ends the list; Enter on it, or a click, shows the next
      page below the rows shown and selects its first row.
17. Up on the first row, Down on the last.
    - The selection goes round to the other end (Up only once the search
      is over).
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

### Preview
21. Click the Preview toggle (off by default) on the Text tab with rows.
    - The list keeps a third of the room (the splitter between them is
      dragged like Find in Files' and remembered) and the preview under it
      shows the selected line's file with its syntax colours and the caret
      after the row's first match, its line in the middle; every
      occurrence of the text in the file is marked (as the reference marks
      the symbol at the caret). The footer keeps the path and "Open In
      Right Split". The toggle is remembered across sessions.
22. Down, Up, or a click on another row.
    - 50 ms after the selection settles the preview follows it: a file row
      at its start (or at the line typed after its name), a text row at its
      line. With the preview on a click selects a row and a double click
      opens it.
23. Click into the preview and type.
    - It takes the focus and shows a caret; once the caret moved only the
      row's matches stay marked (in the found usage's colour). The text
      does not change and "This view is read-only" shows in a balloon
      above the caret's line, its pointer at the caret, until the next key.
24. Select a folder or an action (All tab).
    - The preview shows "There is no preview for the selected item".
      Without a selected row, on "… more" and on the Actions tab there is
      no preview.

## Notes
- The reference sorts its Files tab with a machine-learning model by
  default (Settings | Advanced Settings | Search Everywhere: "Sort results
  in the Files tab based on machine learning", on by default). Pyoxide
  uses the fixed rule of step 5, which gives the top results the reference
  shows; without that setting the reference orders by its own weights,
  where the folder part's match counts and folders go last.
- The reference also has the Classes and Symbols tabs (they come with a
  language server), "Open in Find Tool Window", typo-tolerant
  action search and a "Recent Locations" row; they are not here yet.
- Fuzzy file search is off by default in the reference and is left out.
- A full name includes the project folder's name, as in the reference: in
  a project folder named `flask`, the folder part `flask/` matches every
  folder.

## Source
Observed behaviour of the reference IDE and the IntelliJ Platform
Community sources (see NOTICE).
