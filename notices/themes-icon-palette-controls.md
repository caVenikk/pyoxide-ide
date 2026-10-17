- crates/platform/themes/src/icon_palette.rs (the icon colour palette and
  the check box colours), crates/platform/themes/src/import/
  (reading `icons` > `ColorPalette` and the New UI or classic look of a
  theme), crates/platform/ui/src/icons.rs (recolouring icons),
  crates/platform/ui/src/components.rs (check boxes and tree arrows),
  crates/platform/ui/assets/icons/general/questionMark.svg and
  questionMark_dark.svg
  Adapted from IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2): the colours
  of colorPaletteNewLight, colorPaletteNewDark and the check box
  entries of colorPaletteClassic, and toColorString, of platform/
  platform-impl/src/com/intellij/ide/ui/UITheme.kt; the recolouring rule of
  UiThemePaletteScopeManager.kt and UiThemePaletteCheckBoxScope.kt; the
  New UI test isBasedOnExperimentalTheme of UITheme.kt; the check box
  colours and sizes of platform/platform-resources/src/themes/expUI/icons/
  dark/checkBox*.svg and platform/platform-impl/resources/com/intellij/ide/
  ui/laf/icons/darcula/checkBox*.svg; the run widget icon colours of
  platform/execution-impl/src/com/intellij/execution/ui/
  RedesignedRunWidget.kt; the context help icon platform/icons/src/expui/
  general/questionMark.svg and questionMark_dark.svg (AllIcons.General.
  ContextHelp in the New UI, unchanged copies). Changes: reimplemented in
  Rust; the palette keys are theme roles and the icons are recoloured by
  replacing the colours in their SVG text; the classic tree arrows are
  drawn as filled triangles of our own.
