- crates/platform/terminal/src/integration/ (mod.rs and the scripts in
  zsh/, bash/, fish/, powershell/)
  Adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2), plugins/terminal: the
  injection of src/org/jetbrains/plugins/terminal/runner/
  LocalShellIntegrationInjector.java (ZDOTDIR for zsh, --rcfile without
  --login for bash, --init-command for fish, -File for PowerShell, the
  login shell and user rcfile variables, FIG_TERM, PROCESS_LAUNCHED_BY_CW
  and PROCESS_LAUNCHED_BY_Q) and of resources/shell-integrations/: the
  ZDOTDIR startup files of zsh/zdotdir/ (.zshenv, .zprofile, .zshrc,
  .zlogin, source-original.zsh: the user's files sourced with the original
  ZDOTDIR), the startup files of bash/bash-integration.bash, the DEBUG trap
  that keeps the user's trap and skips PROMPT_COMMAND of
  bash/command-block-support-reworked.bash, the prompt and PSReadLine hooks
  of powershell/powershell-integration.ps1 and
  command-block-support-reworked.ps1, and the sourcing after the user's
  configuration of fish/fish-integration.fish.
  Changes: Pyoxide's own scripts that emit standard OSC 7 and OSC 133
  instead of the reference's OSC 1341 events; the scripts to source are
  passed in numbered variables with their arguments; /bin/sh of macOS gets
  ENV (it ignores --rcfile) with a POSIX script that reads the profile
  files and the user's ENV; no command history, aliases or completion
  events; written to a cache folder per version of their text.
