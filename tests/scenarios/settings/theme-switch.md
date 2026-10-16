# Switch the theme

## Steps
1. In Settings > Appearance & Behavior > Appearance pick Islands Light in
   the Theme combo; press Cancel.
2. In the toolbar gear menu open Theme ▸; move the selection with the
   arrow keys and wait; press Escape.
3. Open Theme ▸ again, move to another theme and press Enter.
4. Press Ctrl+` (Quick Switch Scheme) and choose Theme.
5. Restart the IDE.

## Expected
- Step 1: the whole IDE switches at once, before Apply; Cancel does not
  switch back.
- Step 2: the submenu lists the themes in groups (Islands, New UI,
  Classic, "Custom" for imported ones) with the current one marked and
  "Import Theme…" at the end; about half a second after the selection
  moves the IDE previews that theme; Escape restores the starting theme.
- Step 3: the chosen theme stays.
- Step 4: the same theme list opens.
- Step 5: the chosen theme is used; the settings file stores it.

## Commands
`settings.switch-theme {"id": ...}`, `settings.quick-switch`,
`workbench.set-theme {"id": ...}`, `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/settings-appearance.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`AppearanceConfigurable`, `QuickChangeLookAndFeel`. The theme groups are to
verify against a reference screenshot.
