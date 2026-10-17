- crates/platform/terminal/src/start.rs, shells.rs, customizer.rs
  Translated from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the start rules of
  plugins/terminal/src/org/jetbrains/plugins/terminal/runner/
  LocalTerminalStartCommandBuilder.java (--login on macOS, -i for bash,
  zsh and fish) and LocalOptionsConfigurer.java (working directory order,
  environment order with TERM, TERMINAL_EMULATOR and TERM_SESSION_ID), the
  default shell of TerminalProjectOptionsProvider.kt, LC_CTYPE of
  util/TerminalEnvironment.kt, the shell names of util/ShellNameUtil.java
  and the shell detection of shellDetection/TerminalShellsDetectionUtil.kt
  (folders, names, Windows shells and their names); the customizer hook
  follows LocalTerminalCustomizer.java. Changes: reimplemented in Rust without
  remote environments (Eel), WSL distributions read from the registry, the
  emulator name "Pyoxide".
