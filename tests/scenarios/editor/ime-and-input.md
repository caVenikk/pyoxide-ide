# Text input and input methods

## Steps
1. Type with a dead key (Option+E then E on a US keyboard on macOS) to get
   `é`.
2. Type Japanese with an input method: start a composition, convert, commit.
3. Type Chinese pinyin and pick a candidate from the candidate window.
4. Press Tab with and without a selection; press Enter inside an indented
   block.

## Expected
- Step 1: the dead key shows a marked (underlined) character, which is
  replaced by `é`.
- Steps 2 and 3: the composition is shown inline, underlined, at the caret;
  the candidate window opens next to the caret; committing replaces the
  composition with the chosen text; undo removes the committed text as one
  step.
- Step 4: Tab inserts the indentation of the file (4 spaces for Python by
  default); with a multi-line selection Tab indents the lines; Enter keeps
  the indentation of the current line.

## Commands
`editor.type`, `editor.tab`, `editor.newline`; the composition path is the
platform input handler and is checked manually on each OS.

## Source
Observed behaviour; https://www.jetbrains.com/help/pycharm/using-code-editor.html
