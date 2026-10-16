# Editor > General: Soft Wraps

## Steps
1. Open a Markdown file with a long line.
2. Open Settings, type `soft` in the search field.
3. Turn on "Soft-wrap these files:"; press Apply.
4. Clear "Use the original line's indent for wrapped fragments"; then turn
   it on again and type `x` in "Add additional indent"; press Apply.
5. Type `4` in the indent field, clear "Only show soft-wrap indicators for
   the current line", set the masks to `*.txt`; press OK.

## Expected
- Step 2: Editor > General is selected; the Soft Wraps group shows
  "Soft-wrap these files:" with the masks `*.md; *.txt; *.rst; *.adoc`
  (disabled while the check box is off; "No files" when empty) and the
  comment "Use * and ? as wildcards and ; to separate patterns", "Use the
  original line's indent for wrapped fragments" (on), "Add additional
  indent: [0] symbols" under it, "Only show soft-wrap indicators for the
  current line" (on); the matching labels are lit.
- Step 3: the open Markdown file wraps at once; `[editor.soft-wraps]` of
  `settings.toml` holds `enabled = true` and the masks shown.
- Step 4: the indent field is disabled while the check box is off; Apply
  keeps the page with "Please enter a number from 0 to 99".
- Step 5: the Markdown file no longer wraps; files opened later follow the
  masks.

## Commands
`settings.open {"page": "editor.general"}`, `settings.search`,
`settings.edit` (`soft-wraps`, `files`, `original-indent`, `indent`,
`current-line-only`), `settings.apply`, `editor.state` (`soft_wraps`).

## Source
https://www.jetbrains.com/help/pycharm/settings-editor-general.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`: `EditorOptionsPanel`
(Soft Wraps group), `EditorSettingsExternalizable` (soft wrap masks).
Reference screenshot: S2 (kept locally).
