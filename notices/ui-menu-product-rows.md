- crates/platform/ui/src/menu.rs (rows with a comment line and a large
  icon), crates/platform/settings-ui/src/first_run/view.rs (the folders'
  rows and the lists' width, the import dialog's glow), the icons
  crates/platform/ui/assets/icons/startup/configFile.svg and
  configFile_dark.svg
  The two-line rows with an icon of the import dialog's lists (Other
  Options, the main button's folders) follow IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  plugins/ide-startup/importSettings/src/com/intellij/ide/startup/
  importSettings/chooser/productChooser/ProductChooserRenderer.kt (the
  icon at the top 8 before the name, the comment below it, 8 above and
  below), chooser/ui/UiUtils.kt (the lists at least as wide as the
  buttons, 280) and ConfigAction.kt (the icon of Config or Installation
  Directory). The icon plugins/ide-startup/importSettings/resources/icons/
  configFile.svg is copied as configFile_dark.svg unchanged; configFile.svg
  is the same drawing in the New UI's light icon colours (#6C707E,
  #EBECF0). The reference's product logos are not used: a folder's row
  shows our settings icon. The background picture of the dialog is
  replaced by a glow of our own. Only behaviour, numbers and this icon are
  used; reimplemented in Rust with GPUI.
