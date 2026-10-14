# Reformat code and optimize imports

## Steps
1. With ruff running and its Formatting feature on, write badly formatted
   code and press Cmd+Alt+L (Ctrl+Alt+L).
2. Select a few lines and press Cmd+Alt+L again.
3. Turn ruff Formatting off and repeat step 1.
4. Write unsorted imports, one of them unused, and press Ctrl+Alt+O.
5. Press Cmd+Z after each action.

## Expected
- Step 1: the whole file is formatted in `ruff format` style (indent from
  the code style), as one undo step; an already formatted file is left
  unchanged.
- Step 2: only the selected range is formatted (range formatting).
- Step 3: the IDE's own formatter is used (Pyoxide v1: nothing happens
  without ruff; reported in the status bar).
- Step 4: imports are sorted in ruff's isort order; the unused import is
  not removed.
- Step 5: each action undoes in one step.

## Commands
`code.reformat`, `code.optimize-imports`.

## Source
https://www.jetbrains.com/help/pycharm/reformat-and-rearrange-code.html;
IntelliJ Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`RuffLspClient` (`shouldFormatThisFileExclusivelyByServer`),
`LspFormattingService`, `RuffImportOptimizer`.
