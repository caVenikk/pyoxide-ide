- crates/platform/settings-ui/src/theme_switch.rs, src/pages/appearance.rs,
  crates/platform/themes/src/catalog.rs, crates/platform/workbench/src/
  settings.rs (the Theme and Switch popups, the gear's Theme…),
  keymaps/macos.toml and keymaps/default.toml (Quick Switch Scheme)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the theme switcher of
  platform/platform-impl/src/com/intellij/ide/actions/
  QuickChangeLookAndFeel.java (its rows, the preview 500 ms after the
  selection rests, the starting theme restored when the popup closes
  without a choice) with QuickSwitchSchemeAction.java,
  QuickChangeSchemesAction.java and SettingsEntryPointGroup.java (the
  ellipsis of the gear's rows); the groups of platform/platform-impl/src/com/
  intellij/ide/ui/laf/ThemeListProviderImpl.kt; the Theme and Editor color
  scheme rows of platform/platform-impl/src/com/intellij/ide/ui/
  AppearanceConfigurable.kt (the theme applies at once); the
  ChangeScheme group of platform/platform-resources/src/idea/
  PlatformActions.xml; the keys of QuickChangeScheme in
  platform/platform-resources/src/keymaps/ ($default.xml, Mac OS X
  10.5+.xml); strings of IdeBundle.properties (title.appearance,
  combobox.look.and.feel, combobox.editor.color.scheme,
  combobox.list.custom.section.title, popup.title.switch) and
  ActionsBundle.properties (action.ChangeLaf.text,
  action.QuickChangeScheme.text), and the import balloon's title of
  platform/lang-api/resources/messages/LangBundle.properties
  (notification.title.color.scheme.added). Changes: reimplemented in Rust with
  GPUI; "Import Theme…" (Pyoxide's theme import) takes the place of "Get
  More Themes…"; imported themes are kept as Pyoxide theme files.
