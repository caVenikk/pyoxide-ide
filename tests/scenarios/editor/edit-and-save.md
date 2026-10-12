# Edit and save

## Steps
1. Open `app.py` (LF line separators, UTF-8 without BOM, ends with a
   newline).
2. Type `print("hi")` and Enter at the end of line 1.
3. Save All (Cmd+S on macOS, Ctrl+S elsewhere).
4. Repeat with a file that uses CRLF separators and a UTF-8 BOM.
5. Type into a file, then close its tab without saving.
6. Type into a file and stop typing (no keys, no mouse) for 2 seconds.
7. Type into a file and select another tab.
8. Type into a file and switch to another application.

## Expected
- Step 2: the text appears as typed, the caret follows, the status bar
  caret position updates on every key; the tab shows that the file is
  modified.
- Step 3: the file on disk has the new text; the modified marker goes away.
- Step 4: the saved file still uses CRLF everywhere (also for the new line)
  and still starts with the BOM.
- Step 5: the file is saved when its tab closes; there is no prompt.
- Step 6: the file is saved after 2 seconds without keyboard or mouse
  input; any input starts the 2 seconds again.
- Step 7: the file of the tab that was left is saved.
- Step 8: every modified file is saved when the window loses the focus.
- The reference IDE saves on deactivation by default and offers saving
  after N idle seconds (off by default, 15 s); Pyoxide turns the idle save
  on with 2 s and adds saving on tab switch (ADR 0007).
- A failing write (read-only file) keeps the file modified and reports the
  error in a notification; the text in the editor is not lost.

## Commands
`editor.type {"text": ...}`, `editor.newline`, `workbench.save-all`,
`editor.save`, `tabs.close`, `workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/saving-and-reverting-changes.html
