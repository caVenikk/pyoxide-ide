- crates/platform/terminal-ui/src/tool_window.rs and
  crates/platform/workbench/src/workbench/terminal_window.rs.
  IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  404a109fe630a2b9013f10b71081f87c9dbb32e2 (pycharm/2026.2.3).
  The shell menu grouping and saved session fields follow
  plugins/terminal/frontend/src/com/intellij/terminal/frontend/action/TerminalNewPredefinedSessionAction.kt
  and plugins/terminal/frontend/src/com/intellij/terminal/frontend/toolwindow/impl/TerminalTabsPersistence.kt.
  Changes: implemented in Rust with the GPUI menu and project workspace store.
