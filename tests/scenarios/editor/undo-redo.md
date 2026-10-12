# Undo and redo

## Steps
1. Open a file and type `hello world` in one go.
2. Undo once.
3. Redo once.
4. Type `abc`, move the caret with an arrow key, type `def`, then undo
   twice.
5. Select a word and type over it, then undo.
6. Undo right after opening a file.

## Expected
- Step 2: the whole word run typed in one go disappears with one undo
  (typing is grouped until the caret moves, a pause, or a different kind
  of edit), and the caret returns to where typing started.
- Step 3: the text comes back with the caret after it.
- Step 4: the first undo removes `def`, the second removes `abc`.
- Step 5: undo restores the selected word and the selection itself.
- Step 6: nothing happens; the file stays unmodified.
- A new edit after an undo clears the redo history.
- Undo after Save All makes the file modified again; undoing back to the
  saved state clears the modified marker.

## Commands
`editor.type`, `editor.undo`, `editor.redo`, `editor.move-left`,
`editor.select-word`, `workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/undoing-and-redoing-changes.html
