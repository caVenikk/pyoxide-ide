- crates/platform/workbench/src/settings/terminal.rs,
  crates/platform/settings-import/src/pycharm/options.rs
  The project and application groups, labels, and local terminal option file
  formats follow IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  plugins/terminal/frontend/src/com/intellij/terminal/frontend/settings/TerminalOptionsConfigurable.kt,
  plugins/terminal/resources/messages/TerminalBundle.properties,
  plugins/terminal/src/org/jetbrains/plugins/terminal/TerminalOptionsProvider.kt,
  TerminalProjectOptionsProvider.kt, EnvironmentVariablesDataOptions.kt and
  settings/impl/TerminalLocalOptionsImpl.kt. Reimplemented with Rust and
  GPUI, the application settings store and the personal project workspace.
