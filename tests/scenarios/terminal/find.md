# Find in terminal output

## Steps
1. Run `echo "foo Foo FOO"`; press Cmd+F (macOS) or Ctrl+F; type `foo`.
2. Press Enter twice, then Shift+Enter.
3. Turn on Match Case; turn it off and turn on Regex; type `F.O`.
4. Press Escape; press Escape again.
5. Select `Foo` in the output and press Cmd+F / Ctrl+F.

## Expected
- Step 1: the find bar opens as an overlay at the top right of the
  session (field, match count, up, down, Match Case, Regex, close; no
  whole-words and no replace); three matches are highlighted, the current
  one in a different colour.
- Step 2: the current match moves forward and back; the count follows.
- Step 3: Match Case leaves one match; the regular expression matches.
- Step 4: the first Escape closes the bar and returns the focus to the
  session; the second Escape goes to the shell.
- Step 5: the bar opens with `Foo` as the query.

## Commands
`terminal.find {"text": ...}`, `terminal.find-next`,
`terminal.find-previous`, `terminal.state` (find: query, matches,
current).

## Source
https://www.jetbrains.com/help/pycharm/terminal-emulator.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `plugins/terminal` (search
component, `BLOCK_TERMINAL_SEARCH_ENTRY` colours). The bar layout is to
verify against a reference screenshot.
