- crates/platform/workbench/src/run/tool_window.rs (the empty state),
  src/run/edit_dialog.rs (the tip shown with nothing selected),
  keymaps/macos.toml and keymaps/default.toml (Choose Run Configuration)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the lines, the link and the
  shortcut in brackets of the Run tool window's empty state
  (platform/execution-impl/src/com/intellij/execution/actions/
  ActivateRunToolWindowAction.kt; run.toolwindow.empty.text.* of
  platform/lang-api/resources/messages/LangBundle.properties); the initial
  selection of platform/execution-impl/src/com/intellij/execution/impl/
  RunConfigurable.kt (getInitialSelectedConfiguration,
  selectConfigurableOnShow) and the tip of ProjectRunConfigurationConfigurable.kt
  with status.text.add.new.run.configuration and
  status.text.or.select.run.configuration.to.edit of
  platform/execution/resources/messages/ExecutionBundle.properties; the
  keys of ChooseRunConfiguration in platform/platform-resources/src/keymaps/
  ($default.xml, Mac OS X 10.5+.xml). Changes: reimplemented in Rust; the
  texts are unchanged.
