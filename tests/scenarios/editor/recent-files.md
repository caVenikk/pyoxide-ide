# Recent Files

The reference IDE's Recent Files popup ("switcher"), with its default
keymaps: macOS ("Mac") and Windows/Linux ("Win"). Covered by
`crates/platform/workbench/tests/search_everywhere.rs` (`recent_files`)
and `crates/platform/workbench/tests/keymap.rs` (every binding).

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Recent Files; again: Show edited only | Cmd+E | Ctrl+E | anywhere in the main window |
| Next / previous row | Down / Up | Down / Up | popup |
| Tool windows / files | Left / Right | Left / Right | popup |
| Open the file or show the tool window | Enter | Enter | popup |
| Take the file off the list | Delete, Backspace, Cmd+Backspace | Delete, Backspace | popup |
| Clear the speed search, else close | Escape | Escape | popup |

## Steps and expected behaviour
1. Open `notes.txt`, `pkg/util.py`, `app.py`; press Recent Files.
   - The popup opens centred a quarter down the window: "Recent Files"
     and "Show edited only" with its shortcut in the header, the tool
     windows on the left ("Project"), the files on the right, the current
     file first and the one before it selected. A name shared by two
     listed files shows its folder after it.
2. Type `no`.
   - Both lists keep the rows that match (speed search), the best file
     match selected; Backspace takes the last letter off, Escape clears
     the text. With nothing found, "Press 'Enter' to search in Project":
     Enter opens Go to File with the text.
3. Press Recent Files again.
   - "Show edited only": only the files changed in this session; again,
     all files.
4. Down, Delete.
   - The selected file goes off the list and its tab closes; the popup
     stays.
5. Enter, or a click on a row.
   - The popup closes and the file opens (the tool window shows). Its
     place and size are kept across sessions.

## Notes
- The reference also lists "Recent Locations" (a popup of its own) and
  can be held open with the modifier for Ctrl+Tab switching; Ctrl+Tab is
  a feature of its own.

## Source
Observed behaviour of the reference IDE.
