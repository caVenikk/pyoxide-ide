# The Run console

## Steps
1. Run a script with `input("x? ")`; click into the console, type `42`
   and press Enter; then run one that reads standard input to its end and
   press Cmd+D (macOS) or Ctrl+D.
2. Print `\033[31mred\033[0m`, `\033[1;4mbold underlined\033[0m`,
   `\033[38;5;208m`, `\033[38;2;1;2;3m`, `\033[7minverse`, the same on
   standard error, `"50%\r100%\n"` and `"ab\bc"`.
3. Raise an exception from a function in another file; print a URL.
   Click the path of a `File "...", line N` line, press Cmd+Alt+Down /
   Up (Ctrl+Alt+Down / Up), click the URL.
4. Print more than a megabyte; scroll up while output arrives; press
   Scroll to End; press Clear All.

## Expected
- Step 1: `42` appears in the user input colour (green, italic) at the
  end of the console as it is typed; Enter sends the line; the script
  gets `42`; the process does not echo it again. Send EOF closes the
  standard input.
- Step 2: the colours, bold, underline and inverse are drawn; standard
  error keeps its own ANSI state; only "100%" remains of the progress line;
  "ac" for the backspace.
- Step 3: only the path between the quotes is a link (underlined, link
  colour); a click opens the file at line N with the focus in the editor;
  Up and Down the Stack Trace go from link to link (round the end) and
  open them; a URL opens in the browser.
- Step 4: the oldest lines are dropped beyond 1024 KB; the view follows
  new output while it shows the end and stays put after scrolling up;
  Scroll to End jumps to the end and follows again; Clear All empties the
  console (enabled while it has text).

## Commands
`run.console.type {"text": ...}`, `run.console.send-eof`,
`run.console.previous-link`, `run.console.next-link`,
`run.console.scroll-to-end`, `run.console.clear`, `run.state`
(`tabs[].console.lines[].runs`, `links`, `pending_input`).

## Source
https://www.jetbrains.com/help/pycharm/run-tool-window.html; observed in
PyCharm 2026.2.3; IntelliJ Platform Community (Apache-2.0)
`ConsoleViewImpl`, `EditorHyperlinkSupport`, `AnsiEscapeDecoder`,
`PythonTracebackFilter`, `UrlFilter`, `ConsoleBuffer`.
