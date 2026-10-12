# Large files

## Steps
1. Generate a Python file of 100,000 lines (simple functions, so the
   highlighter has real tokens) and open it.
2. Put the caret in the middle (line 50,000) and type 200 characters.
3. Scroll from the top to the bottom with the scroll wheel and with Page
   Down held.
4. Open a 10 MB text file.
5. Open a file with one line of 5 MB.

## Expected
- Step 1: the file is shown at once; highlighting may arrive a moment later
  but the editor is usable immediately.
- Step 2: every key appears on the next frame (keystroke to paint p99
  within 16 ms, docs/performance.md); highlighting of the edited line
  follows without flicker.
- Step 3: scrolling keeps the display rate (no dropped frames); line
  numbers and highlighting are present on every frame.
- Step 4: open within 0.2 s; scrolling without dropped frames.
- Step 5: the editor stays responsive; the line is shown (long lines may be
  cut off for display beyond a limit, as the reference IDE does).

## Commands
`project.open-file`, `editor.set-selection`, `editor.type`,
`editor.scroll-to {"line": ...}`; the bench mode `PYOXIDE_BENCH` measures
these (replica/perf.md).

## Source
https://www.jetbrains.com/help/pycharm/file-size-limit.html, docs/performance.md
