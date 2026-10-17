- crates/python/pyoxide-ide/src/interpreter/ (table.rs, list.rs,
  configuring.rs, create.rs, candidates.rs, add_dialog.rs, list_dialog.rs,
  page.rs, widget.rs), crates/python/py-env/src/uv.rs, venv.rs, ops.rs,
  interpreter.rs, crates/platform/project/src/idea.rs (the SDK's own data),
  crates/python/pyoxide-ide/assets/icons/homeFolder.svg, moveUp.svg,
  show.svg, greenCheckmark.svg, externalLink.svg, download.svg, edit.svg
  (with their `_dark.svg` variants)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the Interpreter page of
  python/src/com/jetbrains/python/configuration/PyActiveSdkConfigurable.java
  (rows, the combo's list and its separators, the "Add Interpreter" link,
  the disabled state while an interpreter is configured) with python/src/
  com/jetbrains/python/sdk/PySdkRendering.kt (groups, the grey version and
  path) and platform/lang-impl/src/com/intellij/webcore/packaging/
  InstalledPackagesPanel.java (the packages toolbar and columns); the SDK
  data of python/python-sdk/src/com/jetbrains/python/sdk/
  PythonSdkAdditionalData.java and python/python-uv/backend/src/com/
  jetbrains/python/sdk/uv/UvSdkAdditionalData.kt (association, flavour, uv
  path and working folder, read from jdk.table.xml), PySdkSettings.kt;
  the names of python/src/com/jetbrains/python/sdk/PythonSdkType.java
  (suggestSdkName); the uv commands of python/src/com/jetbrains/python/sdk/
  uv/impl/UvLowLevel.kt (init --bare --no-project, venv --clear with the
  --force retry, the Python versions of `uv python list`) and UvExt.kt;
  the location check of python/src/com/jetbrains/python/sdk/add/v2/
  FileSystem.kt (validateVenv); the Add Python Interpreter dialog of
  python/src/com/jetbrains/python/sdk/add/v2/
  PythonAddLocalInterpreterDialog.kt, PythonAddCustomInterpreter.kt,
  CustomNewEnvironmentCreator.kt, venv/EnvironmentCreatorVenv.kt,
  venv/PythonExistingEnvironmentSelector.kt, uv/EnvironmentCreatorUv.kt,
  uv/UvExistingEnvironmentSelector.kt, ValidatedPathField.kt,
  venvExistenceValidationAlert.kt, sortForExistingEnvironment.kt and
  uiUtils.kt (rows, which manager is preselected, the interpreter lists
  and their order, how a row is drawn, the path checks, the alerts and
  their links) with common.kt and venv/helper.kt (the Pythons to download
  and install, installed before the environment is made); the Python
  Interpreters dialog of python/src/com/jetbrains/python/configuration/
  PythonInterpreterConfigurable.kt, PythonInterpreterMasterDetails.kt,
  PythonLocalInterpreterConfigurable.kt, PythonPathDialog.java and
  PythonPathEditor.java with platform/platform-api/src/com/intellij/
  openapi/ui/MasterDetailsComponent.java (toolbar, list order, details,
  Rename and its validation, Apply/OK); the status bar popup's rows of
  PySdkPopupFactory.kt; strings of PyBundle.properties, IdeBundle.properties
  and IdeCoreBundle.properties. Icons: platform/icons/src/expui/nodes/
  homeFolder.svg, expui/general/moveUp.svg, show.svg, greenCheckmark.svg,
  download.svg and edit.svg, expui/ide/externalLink.svg (unchanged copies,
  renamed by dropping their folders). Changes: reimplemented in Rust with
  GPUI; the list of interpreters is kept in Pyoxide's interpreters.toml
  instead of the SDK table and imported from it once; a virtual
  environment can be made with the venv module when uv is missing; the
  Pythons to download are uv's (`uv python list --only-downloads`,
  installed with `uv python install`) instead of the python.org
  installers, and are offered in the uv Python version list too.
