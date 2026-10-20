- crates/python/pyoxide-ide/src/debugger_settings.rs,
  crates/python/py-debug/src/settings.rs
  The page Settings > Python > Debugger, its options and defaults follow
  IntelliJ Community (Apache-2.0), https://github.com/JetBrains/intellij-community
  @ pycharm/2026.2.3: python/src/com/jetbrains/python/debugger/
  PyDebuggerConfigurable.kt (the rows, the Qt backends, the spinner's
  step), PyDebuggerOptionsProvider.java (the options, their defaults and
  names in workspace.xml), PyDebugProcess.java and python/pydevSrc/src/com/
  jetbrains/python/debugger/pydev/RemoteDebugger.java (the evaluation
  response timeout), python/ide/customization.frontend/src/
  PyCharmProjectConfigurableStartupActivity.kt (the page's place in the
  Python group), and the strings of python/pluginResources/messages/
  PyBundle.properties. Reimplemented in Rust with GPUI over Pyoxide's own
  project state; the options pydevd needs and the Mode switch are drawn
  without effect.
