- crates/platform/terminal-ui/src/tool_window.rs, crates/platform/workbench/src/terminal.rs
  (ALLOWED_ACTION_IDS, the key routing), crates/platform/workbench/src/workbench/terminal_window.rs,
  crates/platform/settings/src/terminal.rs (close-session-on-exit,
  override-ide-shortcuts, option-as-meta), keymaps/*/terminal.toml (the
  tool window's keys), crates/platform/terminal/src/running.rs and
  crates/platform/process/src/sys/ (the check for a running command),
  keymaps/actions/window.toml (ShowContent)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  list of actions a terminal leaves to the IDE while it overrides IDE
  shortcuts, and the routing of the other keys to the shell, of
  plugins/terminal/frontend/src/com/intellij/terminal/frontend/view/impl/
  TerminalEventDispatcher.kt (ALLOWED_ACTION_IDS); the default tab names
  of plugins/terminal/src/org/jetbrains/plugins/terminal/util/
  TerminalTitleUtils.kt (createDefaultTabName with UniqueNameGenerator);
  the options of TerminalOptionsProvider (closeSessionOnLogout,
  overrideIdeShortcuts, useOptionAsMetaKey); the confirmation before a
  tab with a running command closes (the texts and buttons of
  platform/execution-impl/src/com/intellij/execution/
  TerminateRemoteProcessDialog.java with ExecutionBundle.properties, the
  check of plugins/terminal/src/org/jetbrains/plugins/terminal/
  TerminalTabCloseListener.kt, TerminalUtil.java hasRunningCommands and
  util/terminalEelProcesses.kt: the shell has a child process); the list
  of tabs of platform/platform-impl/src/com/intellij/ide/actions/
  ShowContentAction.java and openapi/wm/impl/content/SelectContentStep.kt.
  Changes: reimplemented in
  Rust on GPUI; the allowed actions are mapped to Pyoxide commands through
  the action table, and every `terminal.*` command is allowed.
