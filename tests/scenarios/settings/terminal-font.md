# Tools > Terminal > Font Settings

## Steps
1. Open Settings > Tools > Terminal (`settings.open {"page": "terminal"}`).
2. Open the Font combo box and choose another family; open Fallback and
   choose one.
3. Set Size to 12.55, Line height to 1.2 and Column width to 1.05; press
   Apply.
4. Change the editor font on Editor > Font and press Apply.
5. Type `big` in Size and `9` in Line height; press Apply.
6. Change Column width and press "Revert Changes".

## Expected
- Step 1: the page is under the Tools group and holds the Font Settings
  group: `Font:` and `Fallback:` combo boxes on one row, `Size:`, `Line
  height:` and `Column width:` fields on the next. The fields show the font
  the terminal has: the editor font and size until a terminal font is
  chosen, spacings 1.0, Fallback `Monospaced` (the platform's own fallback;
  no font is added behind the user's back).
- Step 2: the lists hold monospaced families only, without groups, each
  family in its own font; the Fallback list starts with `Monospaced`. The
  terminal waits for Apply.
- Step 3: open terminal views lay their text out again at once. Size is
  kept to one digit (12.6), the spacings to two (1.05). `[terminal.font]` of
  `settings.toml` holds the fields that were changed only; the family stays
  unset and keeps following the editor font.
- Step 4: the terminal's family follows the editor font; its size, chosen in
  step 3, stays.
- Step 5: no error: a text that is no number is the default (13.0 for the
  size, 1.0 for a spacing), a number out of range is kept in range (4 to 40;
  0.6 to 3.0).
- Step 6: the field shows the stored value again.

## Commands
`settings.open {"page": "terminal"}`, `settings.edit` (`family`, `fallback`,
`size`, `line-height`, `column-width`, `list`, `choose`), `settings.apply`,
`settings.revert`, `settings.state` (`fields`), `settings.get {"key":
"terminal.font"}`.

## Older configurations
A configuration made before the first-run window has no `[import]` record.
At the first start of a main window with such a configuration the reference's
newest configuration folder is imported once, without asking: its fonts
(`editor-font.xml`, `terminal-font.xml`) fill the keys the settings do not
hold, the snapshot is kept and `[import]` is recorded (every importer counts
as applied). A balloon names what came in.

## Source
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`TerminalOptionsConfigurable`, `TerminalFontSettingsService`,
`TerminalSettingsFloatValueImpl`, `FontComboBox`, `TerminalBundle`.
No reference screenshot of this page yet.
