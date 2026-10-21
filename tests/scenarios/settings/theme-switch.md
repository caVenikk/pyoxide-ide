# Switch the theme

## Steps
1. In Settings > Appearance & Behavior > Appearance pick Islands Light in
   the Theme combo; press Cancel.
2. In the toolbar gear menu choose Theme…; move the selection with the
   arrow keys and wait; press Escape.
3. Open the Theme popup again, move to another theme and press Enter.
   Open it once more and press a digit key.
4. Press Ctrl+` (Quick Switch Scheme) and choose Theme.
5. Restart the IDE.
6. Import a theme plugin with an icon colour palette (Monokai Pro) and
   switch to one of its themes; open Settings > Editor > Font.

## Expected
- Step 1: the whole IDE switches at once, before Apply; Cancel does not
  switch back.
- Step 2: a popup titled Theme in the middle of the window lists the
  themes in groups set apart by lines (Islands, the other built-in themes,
  imported ones) with "Import Theme…" at the end; it opens on the current
  theme. About half a second after the selection rests on a theme the IDE
  previews it; Escape restores the starting theme.
- Step 3: the chosen theme stays. The first ten rows of the popup have
  the grey digits 1 to 9 and 0 in front of their names (the lines and the
  rows after the tenth have none); the digit key chooses its row at once,
  as Enter does (the Switch and Keymap popups are numbered the same way).
- Step 4: a popup titled Switch lists Theme; choosing it opens the same
  Theme popup.
- Step 5: the chosen theme is used; the settings file stores it.
- Step 6: the icons take the theme's colours: the run widget's Run and
  More icons (`RunWidget.*`), folders and source roots in the project tree,
  git status colours of files, the incoming and outgoing arrows of the VCS
  widget, the toolbar and tool window icons. A classic theme (not based on
  a New UI theme) draws filled triangles as tree arrows, 14 px check boxes
  in its palette's check box colours and dialog buttons 24 high; the
  bundled themes draw chevrons, 16 px check boxes and buttons 28 high.

## Commands
`settings.switch-theme {"id": ...}`, `settings.quick-switch`,
`workbench.set-theme {"id": ...}`, `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/settings-appearance.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`AppearanceConfigurable`, `QuickChangeLookAndFeel`, `ThemeListProviderImpl`,
`SettingsEntryPointGroup` (the gear's rows end with an ellipsis and run the
switcher), `QuickChangeSchemesAction`, `UITheme` (icon colour palette),
`RedesignedRunWidget` (run widget icon colours).
