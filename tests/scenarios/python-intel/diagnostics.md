# Problems highlighted in the editor

## Steps
1. With ruff running, type `import os` in a file that never uses `os`.
2. Type `foo()` where `foo` is not defined.
3. Type a line that a warning-level rule reports (for example `x == None`
   with E711 selected) and a type error (`bad: int = "text"`).
4. Hover each highlight; click the rule link in the tooltip.
5. Press F2 repeatedly, then Shift+F2; press Cmd+F1 (Ctrl+F1) on a
   problem.
6. Look at the right edge and the top-right corner of the editor; click
   the inspection widget.
7. Type quickly in the middle of a highlighted range.

## Expected
- Step 1: after the 300 ms highlighting delay plus server time `os` is
  greyed like unused code (tag Unnecessary), without a wave.
- Step 2: red wavy underline (ruff F821 is an error); with a type server
  the same range shows one marker that lists both messages.
- Step 3: yellow wave for the warning; red wave for the type error.
- Step 4: tooltip with the message, then grey `Ruff(` + link
  `unused-import` + `)`; the rule code is shown by its name once rule
  information is loaded (`Ruff(F401)` before that). Clicking the link
  expands the rule documentation in the tooltip.
- Step 5: F2 visits errors before warnings, wrapping at the end; Shift+F2
  goes back; Cmd+F1 shows the description tooltip at the caret.
- Step 6: one stripe mark per problem line in the severity colour (at
  least 2 px high); the widget shows counts per severity with icons, a
  check mark when the file is clean; clicking it opens the Problems tool
  window (Pyoxide v1: goes to the first problem until the Problems window
  exists).
- Step 7: highlights move with the text; stale ones are replaced when the
  server answers for the new version; typing latency stays within budget.

## Commands
`code.next-problem`, `code.previous-problem`, `code.error-description`,
`editor.state` (`diagnostics[]`: range, severity, message, source, code).

## Source
https://www.jetbrains.com/help/pycharm/code-inspection.html; ruff
https://docs.astral.sh/ruff/editors/features/; IntelliJ Platform Community
(Apache-2.0) at `pycharm/2026.2.3`: `LspDiagnosticsCustomizer`,
`PyLspToolDiagnosticsSupport`, `RuffLspClient`, `RuffTooltipLinkHandler`,
`DaemonCodeAnalyzerSettings`, `LspHighlightingPass`.
