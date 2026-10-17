# Editor > Font

## Steps
1. Open a file and run it, so an editor and the Run console are open.
2. Open Settings > Editor > Font (⌘, then the tree, or
   `settings.open {"page": "editor.font"}`).
3. Open the Font combo box; choose another family.
4. Set Size to 20 and Line height to 1.5; turn on Enable ligatures; type in
   the preview.
5. Press Apply.
6. Type `big` in Size and press Apply.
7. Click "Restore defaults", then OK.

## Expected
- Step 2: Font, Size and Line height on two rows (`Size:` `13.0`,
  `Line height:` `1.2` with the bundled themes), Enable ligatures with its
  circled "?" help icon, under it the comment "See line height and
  ligatures also in Reader mode" (the link selects the Reader mode page
  where one is registered), "Restore defaults" drawn disabled while the font is
  JetBrains Mono at 13 with line height 1.2 and no ligatures, the
  collapsed Typography Settings (Fallback font: `<None>` with its comment),
  and on the right the editable preview with "Enter any text to preview"
  under it.
- Step 3: the families come from the text system, listed in the background
  (the list opens at once; it fills when the listing is done): "Monospaced"
  first, then "Proportional"; each family in its own font. A family the
  system lacks shows as `<Font missing: …>`.
- Step 4: the preview follows each change; the editors and the console do
  not; Apply is enabled and "Restore defaults" is a link.
- Step 5: every editor and the Run console lay their text out in the new
  font; `[editor.font]` of `settings.toml` holds what differs from the
  theme's font.
- Step 6: the page stays selected with the error "Please enter a number
  from 4 to 40"; nothing changes.
- Step 7: the theme's font again; nothing is stored in `[editor.font]`.

## Commands
`settings.open {"page": "editor.font"}`, `settings.edit` (`family`, `size`,
`line-height`, `ligatures`, `fallback`, `restore-defaults`, `typography`,
`list`, `choose`), `settings.apply`, `settings.state` (`fields`),
`settings.get {"key": "editor.font"}`.

## Source
https://www.jetbrains.com/help/pycharm/settings-editor-font.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `AppEditorFontConfigurable`,
`AppFontPanel`, `AppFontOptionsPanel`, `AppEditorFontOptionsPanel`,
`FontFamilyCombo`, `FontPreviewService`, `EditorFontsConstants`. Reference
screenshot: S5 (kept locally).
