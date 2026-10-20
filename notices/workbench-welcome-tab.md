- crates/platform/workbench/src/welcome.rs,
  crates/platform/workbench/src/workbench/welcome.rs, the icons
  crates/platform/ui/assets/icons/general/upload.svg, upload_dark.svg,
  keyboard.svg, keyboard_dark.svg and meetNewUi/darkTheme.svg,
  darkTheme_dark.svg, lightTheme.svg, lightTheme_dark.svg
  The Welcome tab (its title and subtitle, the tiles, the Theme and Keymap
  combo boxes and their lists) follows IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  platform/non-modal-welcome-screen/src/com/intellij/platform/ide/
  nonModalWelcomeScreen/rightTab/ (WelcomeScreenRightTab.kt: the layout,
  sizes, gaps and the tiles' colours; WelcomeScreenRightTabComboBoxModel.kt:
  what the lists hold; components/WelcomeScreenCustomListComboBox.kt: the
  list's minimum width), the texts of platform/non-modal-welcome-screen/
  resources/messages/NonModalWelcomeScreenBundle.properties and
  python/common/resources/messages/PyWelcomeBundle.properties, and the
  Help menu's Welcome of platform/non-modal-welcome-screen/resources/
  intellij.platform.ide.nonModalWelcomeScreen.xml. The icons are copied
  unchanged from platform/icons/src/expui/ (general/upload, general/keyboard,
  meetNewUi/darkTheme, meetNewUi/lightTheme). Only behaviour, numbers,
  texts and these icons are used; reimplemented in Rust with GPUI. The
  background picture is replaced by a glow of our own, the product name
  is ours, and the tiles are the ones Pyoxide has.
