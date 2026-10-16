# Soft wraps

## Steps
1. Open a file with a few lines longer than the editor is wide, one of
   them indented, and some short lines.
2. View > Active Editor > Soft-Wrap (or the command `editor.toggle-soft-wraps`).
3. Make the window narrower and wider.
4. Put the caret on a wrapped line and move with Up and Down, with and
   without Shift.
5. Press End twice on the first row of a wrapped line, then Home three
   times on a later row.
6. Click past the end of a wrapped row; click in the indent of a
   continuation row.
7. Type at the end of a short line until it no longer fits; undo.
8. Turn Soft-Wrap off again.
9. With "Soft-wrap these files" on for `*.md` (`editor.toml` in the
   configuration folder: `[soft_wraps] enabled = true`, `files = "*.md"`),
   open a Markdown file and a Python file.
10. Turn "use original line's indent" off (`use_original_indent = false`),
    then give it 2 extra columns (`use_original_indent = true`,
    `indent = 2`); turn "show all soft wraps" on (`show_all = true`).

## Expected
- A line wider than the view goes on in the next visual rows; nothing
  goes under the vertical scrollbar, and there is no horizontal scrolling.
- A row ends after a space within ten characters before the room kept
  for the wrap sign; a long word without spaces breaks at a comma, a
  bracket or a dot, else at the edge.
- Continuation rows start one column (the sign) plus the line's
  indentation to the right.
- Line numbers show on the first row of a line only.
- Up and Down move by visual rows and keep the column the caret had.
- End goes to the end of the caret's row (the caret stays on that row),
  a second End to the end of the line. Home on a continuation row goes
  to the row's start, then to the first non-blank character of the line,
  then to column 1.
- A click past a row's end puts the caret at that end on that row.
- Edits wrap their lines again at once; the view keeps the line at its
  top when the width or the Soft-Wrap state changes.
- Large files are wrapped without blocking typing or scrolling.
- Files matching the masks open with soft wraps; others do not.
- On the caret's line a sign (⤦) follows the last char of each row before
  a wrap and a sign (⤥) stands in the last column of the next row's
  indent, in the scheme's soft wrap sign colour; with "show all soft
  wraps" on every wrapped line.
- Without the original indent continuation rows start after the sign
  only; with an extra indent that many columns further right.
- The caret row highlight covers the caret's visual row only.
- A Python line without spaces near the edge wraps after a comma or an
  opening bracket.

Not yet: the settings page (the options are in `editor.toml`), the
console's own Soft-Wrap toggle (#77 uses `Editor::set_soft_wraps`), the
caret stop at column 1 of a continuation row's indent (Home goes from the
row's start straight to the line's text), the Python wrap rules that
look at strings and f-strings. Needs a reference screenshot: the sign
colour of the Islands schemes (the whitespace colour for now).

## Commands
`editor.toggle-soft-wraps`, `editor.move-up`, `editor.move-down`,
`editor.select-down`, `editor.move-line-start`, `editor.move-line-end`,
`editor.select-line-end`, `editor.scroll-to`, `workbench.state`
(`editor.soft_wraps`, `editor.rows`, `editor.caret_row`,
`editor.wrap_columns`).

## Source
https://www.jetbrains.com/help/pycharm/using-code-editor.html (soft wraps),
IntelliJ Platform Community at tag pycharm/2026.2.3 (see NOTICE).
