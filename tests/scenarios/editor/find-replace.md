# Find and replace in the file

The find bar of the reference IDE's editor, with its default keymaps:
macOS ("Mac") and Windows/Linux ("Win"). Keystrokes come from the IntelliJ
Platform Community keymap files; option and button keys are the mnemonics
of their labels (see NOTICE). Covered by
`crates/platform/workbench/tests/find.rs` (keystrokes through both
keymaps), `crates/platform/workbench/tests/keymap.rs` (every binding),
`crates/platform/search` (matching and replacement) and the unit tests of
`crates/platform/editor/src/find.rs`.

## Keys
| Action | Mac | Win | Where |
|---|---|---|---|
| Find | Cmd+F | Ctrl+F, Alt+F3 | editor, find bar |
| Replace | Cmd+R | Ctrl+R | editor, find bar |
| Find Next | Cmd+G | F3, Ctrl+L | editor, find bar |
| Find Previous | Cmd+Shift+G | Shift+F3, Ctrl+Shift+L | editor, find bar |
| Next / previous match | Enter, Down, Ctrl+N / Shift+Enter, Up, Ctrl+P | Enter, Down / Shift+Enter, Up | find bar fields |
| Close the bar | Escape | Escape | find bar fields |
| Back to the editor | Cmd+Enter | Ctrl+Enter | find bar fields |
| Other field | Tab, Shift+Tab | Tab, Shift+Tab | find bar fields |
| Search history | Option+Down | Alt+Down | find bar fields |
| Match Case / Words / Regex | Ctrl+Option+C / W / X, Option+C / W / X | Alt+C / W / X | find bar fields |
| In Selection | Ctrl+Option+E | Ctrl+Alt+E | find bar fields |
| Replace / Replace All / Exclude | Ctrl+Option+P / A / L | Alt+P / A / L | find bar fields |
| Replace (current) | Enter | Enter | replace field |

On macOS F3 is not Find Next: the macOS keymap binds Find Next to F3 and
then again to Cmd+G, and the later entry replaces the earlier one.

## Steps and expected behaviour

### Opening the bar
1. Select `two` in `one two two` and press Find.
   - The bar opens at the top of the editor, below the tabs, with `two`
     in the search field, selected, and the focus in the field. Both
     matches are highlighted; the selected one is the current match,
     counter `1/2`.
2. Type `one`: it replaces the field text.
3. Press Escape: the bar closes, the highlighting goes, `one` stays
   selected and the editor has the focus.
4. Put the caret somewhere without a selection and press Find: the field
   shows the last search (`one`), selected. The word at the caret is not
   used (the reference IDE does that only with a registry switch that is
   off by default).
5. Select a few lines and press Replace: the bar opens in replace mode
   with an empty search field ("Search in selection only"); matches are
   searched only within the lines, which stay selected, and the current
   match is outlined with the active match background. Find with a
   selection over several lines does the same here (the reference IDE puts
   the lines into a multi-line search field instead).
6. With Regex on, a selected `a.b` goes into the field as `a\.b`.

### Searching while typing
1. In `alpha beta alpha gamma alpha` with the caret at the start, press
   Find and type `a`, then `lpha`.
   - Every key searches again; all matches are highlighted and marked on
     the scrollbar; the current match is the one at (or after) the caret,
     selected, and scrolled into the middle of the view when it was not
     visible; it stays the current match while the query grows.
   - The counter reads `1/3`; without a current match `3 results`; with
     more than 1,000,000 matches `1000000+`.
2. Type a query that is not in the file: `0 results`, the field text and
   the counter in the error colour.
3. Clear the field: the selection the bar opened with comes back.
4. Regex on, type `(`: `Bad pattern` in the error colour, no highlighting,
   no crash. A pattern of bars only (`|`, `||`): `Vague pattern`.

### Moving between matches
1. Enter (or Down) selects the next match, Shift+Enter (or Up) the
   previous one; the counter follows (`2/3`).
2. Find Next / Find Previous do the same from the editor while the bar
   is open.
3. At the last match, Find Next does not move: a hint below the caret says
   `"alpha" not found, press ⌘G to search from the top` (the key of Find
   Next on the platform; F3 on Windows/Linux). Pressing it again selects
   the first match. Find Previous at the first match says `... to search
   from the bottom` and then goes to the last match.
4. Any key hides the hint; moving the caret elsewhere ends the pending
   wrap.
5. Clicking into another match makes it the current match (the selection
   stays as clicked).

### Find Next with the bar closed
1. Without any search in the session, Find Next opens the bar.
2. After a search and Escape, Find Next selects the next match of the last
   search after the selection, without highlighting; at the end the same
   hint, then from the top. Find Previous the same way backwards.

### Options
1. Match Case, Words and Regex are toggle icons inside the search field
   (Aa, W, .*); the keys switch them while the focus is in the bar; the
   focus stays in the field and the search runs again.
2. Words: a match is not part of a longer word (letters and digits of any
   script and `_` are word characters); `==` is not found in `===`.
3. Regex uses `^` and `$` per line; `.` does not match a line break.
4. The options stay for the next bar (in any editor) of the session.

### Replacing
1. Press Replace: the replace row appears below the search field, with
   Replace, Replace All and Exclude (Include for an excluded match). The
   mode chevron left of the search field switches between Find and
   Replace.
2. Replace (or Enter in the replace field) replaces the current match and
   selects the next one.
3. With Regex, the replacement takes groups: `(\w)=(\d)` replaced with
   `$2=$1` turns `a=1` into `1=a`; `${name}`, `\n`, `\t`, `\u`, `\U...\E`
   work as well. A malformed replacement (`$9` with one group) is reported
   and nothing changes.
4. Exclude leaves the current match out (struck through) and goes to the
   next one; Replace All replaces every other match.
5. Replace All is one undo step; afterwards the caret is after the last
   replacement and a hint says `2 occurrences replaced`. Within a
   selection the selection grows or shrinks with the replacements.

### History
1. Searches used with next/previous, Replace or Ctrl/Cmd+Enter are kept
   for the session (newest first, 30 each for searches and replacements).
2. Alt/Option+Down in a field shows its history below it; Up and Down move,
   Enter (or a click) puts the item into the field and searches, Escape
   closes the list.

### Keyboard layouts
With a non-Latin input source (Russian) on macOS, Cmd+F, Cmd+G, Cmd+R and
the Option toggles still work: the key of the Command layout is used for
shortcuts, and Cyrillic text is typed into the field.

### Large files
On the 10 MB file of the performance runs, typing a query keeps the editor
responsive: each key starts a search on a snapshot of the text off the UI
thread and cancels the previous one; the matches appear within a few
frames (replica/perf.md). Files up to 256 KB are searched at once, so the
frame of the key already shows the matches.

## Not covered yet
Multi-line search and replace fields, Preserve Case, the search filters
(comments, string literals), Select All Occurrences and the other
multiple-caret actions of the bar, Open in Find Window, the regex help
popup, replacement preview balloons.

## Source
https://www.jetbrains.com/help/pycharm/finding-and-replacing-text-in-file.html,
the IntelliJ Platform Community keymap files and find bar behaviour (see
NOTICE).
