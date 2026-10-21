- crates/platform/workbench/src/welcome.rs, welcome/projects.rs,
  crates/platform/workbench/src/workbench/welcome.rs, the icons
  crates/platform/ui/assets/icons/general/upload.svg, upload_dark.svg,
  keyboard.svg, keyboard_dark.svg, learn.svg, learn_dark.svg,
  nodes/plugin.svg, plugin_dark.svg and meetNewUi/darkTheme.svg,
  darkTheme_dark.svg, lightTheme.svg, lightTheme_dark.svg
  The Welcome tab (its title and subtitle, the tiles, the Theme and Keymap
  combo boxes and their lists) follows IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  platform/non-modal-welcome-screen/src/com/intellij/platform/ide/
  nonModalWelcomeScreen/rightTab/ (WelcomeScreenRightTab.kt: the layout,
  sizes, gaps and the tiles' colours; WelcomeScreenRightTabComboBoxModel.kt:
  what the lists hold; components/WelcomeScreenCustomListComboBox.kt: the
  list's minimum width; components/WelcomeScreenCustomButton.kt: the
  focused tile), the Projects and Files panel of platform/
  non-modal-welcome-screen/src/com/intellij/platform/ide/
  nonModalWelcomeScreen/leftPanel/ (WelcomeScreenLeftPanel.kt,
  WelcomeScreenLeftPanelActions.kt, WelcomeScreenLeftTabActionNew.kt: the
  buttons, lines, search field and empty text) with the metrics of
  platform/platform-impl/src/com/intellij/openapi/wm/impl/welcomeScreen/
  WelcomeScreenVerticalToolbar.kt, platform/platform-impl/src/com/
  intellij/ui/components/DisclosureButton.kt and ide/ui/laf/darcula/ui/
  DarculaDisclosureButtonUI.kt, DarculaDisclosureButtonBorder.kt, and
  platform/platform-api/src/com/intellij/util/ui/StatusText.java (the
  empty text a third of the way down), the list colours of platform/
  jewel/ide-laf-bridge/src/main/kotlin/org/jetbrains/jewel/bridge/theme/
  IntUiBridgeSimpleListItem.kt, the texts of platform/non-modal-welcome-screen/
  resources/messages/NonModalWelcomeScreenBundle.properties and
  python/common/resources/messages/PyWelcomeBundle.properties and
  platform/platform-resources-en/src/messages/ActionsBundle.properties
  (Open…, Clone Repository…), and the Help menu's Welcome of
  platform/non-modal-welcome-screen/resources/
  intellij.platform.ide.nonModalWelcomeScreen.xml. The icons are copied
  unchanged from platform/icons/src/expui/ (general/upload,
  general/keyboard, general/learn, nodes/plugin, meetNewUi/darkTheme,
  meetNewUi/lightTheme). Only behaviour, numbers, texts and these icons
  are used; reimplemented in Rust with GPUI. The
  background picture is replaced by a glow of our own, the product name
  is ours, and the tiles and buttons without a feature behind them yet
  are shown dimmed.
