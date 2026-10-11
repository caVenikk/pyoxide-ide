# Caret movement and selection

## Steps
1. Open a file with lines of different lengths, tabs and non-ASCII text
   (`naïve`, CJK, an emoji).
2. Move with Left/Right, Up/Down, word moves (Alt+Left/Right on macOS,
   Ctrl+Left/Right elsewhere), line start/end (Cmd+Left/Right on macOS,
   Home/End elsewhere), document start/end (Cmd+Home/End on macOS,
   Ctrl+Home/End elsewhere), Page Up/Down.
3. Repeat each move with Shift held.
4. Click in the text, drag, double click a word, triple click a line,
   Shift+click after a caret.
5. Select all.

## Expected
- Moves step over whole characters (a combined emoji or accented letter
  is one step), never into the middle of a character.
- Up and Down keep the column the caret had before (preferred column)
  across shorter lines.
- Word moves stop at word starts and ends; punctuation and spaces are
  their own stops, as in the reference IDE.
- Home toggles between the first non-blank character and column 1.
- With Shift the selection grows from the anchor and shrinks back when
  moving the other way.
- Mouse: click places the caret, drag selects, double click selects a word,
  triple click selects the line including its separator, Shift+click
  extends from the caret.
- The status bar shows the caret position as `line:column` and, with a
  selection, the number of selected characters (and line breaks).

Note: on macOS Cmd+Up and Cmd+Down are navigation actions in the reference
keymap, not document moves.

## Commands
`editor.move-left`, `editor.move-right`, `editor.move-up`, `editor.move-down`,
`editor.move-word-left`, `editor.move-word-right`, `editor.move-line-start`,
`editor.move-line-end`, `editor.move-document-start`,
`editor.move-document-end`, `editor.page-up`, `editor.page-down`, each with a
`select-` variant, `editor.select-all`, `editor.set-selection`,
`workbench.state`.

## Source
https://www.jetbrains.com/help/pycharm/using-code-editor.html,
https://www.jetbrains.com/help/pycharm/mastering-keyboard-shortcuts.html
