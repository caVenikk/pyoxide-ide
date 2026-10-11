# Edit and save

## Steps
1. Open `app.py` (LF line separators, UTF-8 without BOM, ends with a
   newline).
2. Type `print("hi")` and Enter at the end of line 1.
3. Save All (Cmd+S on macOS, Ctrl+S elsewhere).
4. Repeat with a file that uses CRLF separators and a UTF-8 BOM.
5. Type into a file, then close its tab without saving.

## Expected
- Step 2: the text appears as typed, the caret follows, the status bar
  caret position updates on every key; the tab shows that the file is
  modified.
- Step 3: the file on disk has the new text; the modified marker goes away.
- Step 4: the saved file still uses CRLF everywhere (also for the new line)
  and still starts with the BOM.
- Step 5: the file is saved when its tab closes; there is no prompt.
- A failing write (read-only file) keeps the file modified and reports the
  error in a notification; the text in the editor is not lost.

## Commands
`editor.type {"text": ...}`, `editor.newline`, `workbench.save-all`,
`editor.save`, `tabs.close`, `workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/saving-and-reverting-changes.html
