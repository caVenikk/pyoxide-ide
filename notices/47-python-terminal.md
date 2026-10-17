- crates/python/pyoxide-ide/src/terminal.rs
  Translated from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the activation rules of
  python/terminal/backend/src/com/intellij/python/terminal/
  PyVirtualEnvTerminalCustomizer.kt (no activation with shell integration
  off or the setting off, the interpreter of the working directory's
  module, a script to source per shell, the variables for other shells),
  the script names of python/python-sdk/src/com/jetbrains/python/sdk/
  PythonEnvironment.kt, the variable names of python/python-sdk/src/com/
  jetbrains/python/run/ActivatableScriptExt.kt, and the file, component
  and option of python/terminal/shared/src/PyVirtualEnvTerminalSettings.kt.
  Changes: reimplemented in Rust; the variables of an activation are set
  as the activation script sets them instead of running it; uv's
  activate.ps1 is found on case-sensitive file systems; conda is not
  activated yet.
