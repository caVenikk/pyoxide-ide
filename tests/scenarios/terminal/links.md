# Links in the terminal

## Steps
1. Run `echo https://example.com` and click the URL.
2. Run `echo src/app.py:3:5` in the project root (the file exists).
   Hover the path, then hold Cmd (macOS) or Ctrl and hover it, then
   Cmd/Ctrl+click it.
3. Click the same path without a modifier and wait.
4. Run a Python script that raises an exception in a function.
5. Run `echo /tmp` and Cmd/Ctrl+click it; `cd src` and run
   `echo app.py:1`.

## Expected
- Step 1: the URL is underlined with a hand cursor; a plain click opens
  it in the browser.
- Step 2: the path has no decoration until Cmd/Ctrl is held; then it is
  underlined with a hand cursor; Cmd/Ctrl+click opens `app.py` with the
  caret at line 3, column 5.
- Step 3: after about 180 ms a hint at the click shows "Open" and
  "⌘Click" / "Ctrl+Click".
- Step 4: each `File "...", line N` of the traceback is a visible link
  that opens the file at the line.
- Step 5: a folder outside the project opens in the system file manager;
  the relative path resolves against the tracked working directory.

## Commands
`terminal.type {"text": ...}`, `terminal.open-link {"line": ..., "column": ...}`
(the link at a cell of the screen), `terminal.state` (`links` on visible
lines, `link_hint`), `workbench.state` (active editor, caret, project tree
selection).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal`
(hyperlinks), `InvisibleHyperlinkHintManager`, `PyMessageFilterProvider`.
The hint look is to verify against a reference screenshot.
