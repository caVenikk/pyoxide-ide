- crates/platform/workbench/src/settings/terminal.rs,
  crates/platform/settings/src/terminal.rs
  The font group of Settings > Tools > Terminal, its labels, the
  monospaced-only families, the sizes, the rounding and the formatting of
  the numbers, and what an unset key inherits follow IntelliJ Community
  (Apache-2.0), https://github.com/JetBrains/intellij-community @
  pycharm/2026.2.3: plugins/terminal/frontend/src/com/intellij/terminal/
  frontend/settings/TerminalOptionsConfigurable.kt, plugins/terminal/src/
  org/jetbrains/plugins/terminal/TerminalFontSettingsService.kt,
  plugins/terminal/src/org/jetbrains/plugins/terminal/util/
  TerminalSettingsFloatValueImpl.kt, plugins/terminal/resources/messages/
  TerminalBundle.properties, platform/platform-impl/src/com/intellij/ui/
  FontComboBox.java and platform/ide-core/resources/messages/
  OptionsBundle.properties (the Tools group). Reimplemented in Rust with
  GPUI over Pyoxide's own settings store.
