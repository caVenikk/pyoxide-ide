# Clipboard

## Steps
1. Select `world` in `hello world` and copy, then paste at the end of the
   line.
2. Put the caret in a line without a selection and copy, then move to
   another line and paste.
3. Cut without a selection.
4. Copy text with CRLF separators from another application and paste it
   into an LF file.

## Expected
- Step 1: the line reads `hello worldworld`; the caret is after the pasted
  text.
- Step 2: without a selection, copy takes the whole line including its
  separator; pasting it inserts the line above the caret line.
- Step 3: the caret line is removed and is on the clipboard.
- Step 4: the pasted text uses the file's line separator.

## Commands
`editor.copy`, `editor.cut`, `editor.paste`, `editor.set-selection`,
`workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/working-with-source-code.html
