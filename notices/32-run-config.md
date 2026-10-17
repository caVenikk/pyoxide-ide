- crates/platform/run-config, crates/python/py-run/src/config.rs,
  crates/platform/workbench/src/run/dropdown.rs, src/run/edit_dialog.rs,
  crates/platform/ui/src/menu.rs (row titles, secondary texts, row
  buttons, rows that run an action and open a submenu),
  docs/adr/0012-run-configuration-storage.md
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the splitting and joining
  of parameter strings of platform/util/src/com/intellij/util/execution/
  ParametersListUtil.java (parse, join, escape); the environment file
  syntax of platform/execution-impl/src/com/intellij/execution/envFile/
  EnvFileParser.kt; the natural order of platform/util/base/src/com/
  intellij/openapi/util/text/NaturalComparator.java; the list order, the
  custom order and the temporary limit of platform/execution-impl/src/com/
  intellij/execution/impl/RunConfigurationListManagerHelper.kt and
  RunManagerImpl.kt, the unique names of platform/execution/src/com/
  intellij/execution/RunManager.kt (suggestUniqueName, extractBaseName);
  the dropdown structure and the name trimming of platform/execution-impl/
  src/com/intellij/execution/ui/RunToolbarPopup.kt and RedesignedRunWidget.kt
  (splitCharCount); the field names and defaults read from the reference's
  run configuration files (RunnerAndConfigurationSettingsImpl.kt,
  python/src/com/jetbrains/python/run/AbstractPythonRunConfiguration.java,
  PythonRunConfiguration.java, python/src/com/jetbrains/python/sdk/uv/run/
  UvRunConfiguration.kt, plugins/sh/core/src/com/intellij/sh/run/
  ShRunConfiguration.java) and the conversion of the standalone uv run
  type of python/src/com/jetbrains/python/run/
  LegacyUvRunConfigurationConverter.kt (master); the dropdown rows, their
  buttons and submenus of RunToolbarPopup.kt and RunConfigurationsComboBoxAction.java
  and the deletion question of ChooseRunConfigurationManager.java; the
  dialog's tree, toolbar and labels of platform/execution-impl/src/com/
  intellij/execution/impl/RunConfigurable.kt and the Python form's fields
  and options of python/src/com/jetbrains/python/run/
  PythonConfigurationFragmentedEditor.kt, the form's layout (head
  fragments, labelled rows, tags, "Modify options" with its shortcut, the
  Run button with the executors) of platform/execution-impl/src/com/
  intellij/execution/ui/RunConfigurationFragmentedEditor.java,
  FragmentedSettingsEditor.java and SettingsEditorFragment.java, the
  dialog window of platform/platform-api/src/com/intellij/openapi/ui/
  DialogWrapper.java (crates/platform/ui/src/dialog_window.rs), the tree's
  initial expansion of RunConfigurable.kt (selectConfigurableOnShow) and
  the expanded paths of its fields (PathMacroManager), the "Add Run
  Options" rows, groups and mnemonics of platform/execution-impl/src/com/
  intellij/execution/ui/CommonTags.java, CommonParameterFragments.java,
  BeforeRunFragment.java and diagnostic/logging/LogsGroupFragment.java,
  the option tags of TagButton and the link of DropDownLink, with the
  labels of
  ExecutionBundle.properties, ActionsBundle.properties, PyBundle.properties
  and ShBundle.properties. Changes: reimplemented in Rust; the natural
  order works on characters, the name trimming counts characters instead
  of measuring widths.
