- crates/platform/settings-ui/src/first_run.rs and first_run/ (view.rs,
  import.rs), crates/platform/settings-import/src/pycharm/options.rs,
  crates/platform/settings-import/src/snapshot.rs,
  crates/platform/workbench/src/settings/first_run.rs, the FirstRun keys
  of crates/platform/workbench/keymaps/macos.toml and default.toml
  The first-run window (Import Settings, Other Options, Importing
  settings, Choose Theme, Choose Keymap) and the reading of the reference
  IDE's configuration folders follow IntelliJ Platform Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  plugins/ide-startup/importSettings/src/com/intellij/ide/startup/
  importSettings/ (chooser/ui/OnboardingDialog.kt, WizardPagePane.kt,
  RoundedPanel.kt, UiUtils.kt, chooser/productChooser/
  ProductChooserPage.kt, chooser/importProgress/ImportProgressPage.kt,
  wizard/themeChooser/ThemeChooserPage.kt, SchemePane.kt,
  wizard/keymapChooser/KeymapChooserPage.kt, KeymapPane.kt: the pages,
  their sizes, gaps, colours and order; jb/JbImportServiceImpl.kt and
  jb/NameMappings.kt: which folders are offered, the dated option files,
  the theme and keymap names), the texts of plugins/ide-startup/
  importSettings/resources/messages/ImportSettingsBundle.properties and
  platform/service-container/resources/messages/BootstrapBundle.properties,
  platform/platform-impl/src/com/intellij/ide/ui/laf/darcula/ui/
  OnboardingDialogButtons.kt (the button colours), platform/
  platform-impl/initial-config-import/src/com/intellij/openapi/application/
  ConfigImportHelper.java (old folders, the folder name pattern, the
  plugin folder), platform/platform-impl/src/com/intellij/ide/ui/laf/
  LafManagerImpl.kt (deprecated theme ids) and platform/platform-impl/src/
  com/intellij/configurationStore/storeUtil.kt (the per-OS option
  folder). The option file formats (laf.xml, colors.scheme.xml,
  keymap.xml, editor-font.xml, editor.xml, ui.lnf.xml, ide.general.xml,
  filetypes.xml) and the password safe's file names that are never copied
  (security.xml, c.kdbx, c.pwd) are read from the components that write
  them. Only behaviour, numbers and texts are used; reimplemented in Rust
  with GPUI. A theme card shows code in the theme's colours instead of a
  picture, product names are replaced by "Reference IDE", and the import
  keeps a snapshot of the data files for features that come later.
