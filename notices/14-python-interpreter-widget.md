- crates/python/pyoxide-ide/src/interpreter.rs and interpreter/widget.rs,
  crates/python/py-env/src/venv.rs, crates/platform/ui/src/menu.rs (list
  popups, speed search), crates/platform/project/src/workspace.rs and
  idea.rs (the project's SDK, the SDK table)
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  interpreter widget of python/ide/impl/src/com/intellij/pycharm/community/
  ide/impl/configuration/PySdkStatusBar.kt with python/src/com/jetbrains/
  python/packaging/widget/PyStatusBarWidgetUtils.kt and platform/
  platform-impl/src/com/intellij/openapi/wm/impl/status/
  EditorBasedStatusBarPopup.kt and TextPanel.kt (text, tooltip, icon, when
  it shows, the popup's position); its popup of python/src/com/jetbrains/
  python/sdk/PySdkPopupFactory.kt and PySdkRendering.kt (rows, groups,
  order, preselection); the presentation of python/python-sdk/src/com/
  jetbrains/python/sdk/PythonInterpreterPresentation.kt and impl/
  PySdkPresentationInfoImpl.kt (`name [X.Y.Z]`, shortening); the names
  and icons of python/src/com/jetbrains/python/sdk/PythonSdkType.java,
  python/python-venv/src/com/intellij/python/venv/sdk/flavors/
  VirtualEnvSdkFlavor.kt and python/python-uv/backend/src/com/jetbrains/
  python/sdk/uv/UvSdkFlavor.kt, with platform/lang-impl/src/com/intellij/
  openapi/projectRoots/impl/SdkConfigurationUtil.java and platform/util/
  src/com/intellij/util/text/UniqueNameGenerator.java (numbered names); the
  environments of a project of python/openapi/src/com/jetbrains/python/
  venvReader/VirtualEnvReader.kt, python/python-sdk/src/com/jetbrains/
  python/sdk/configuration/PyProjectSdkConfigurationExtension.kt and sdk/
  impl/PythonEnvironmentDetector.kt, python/src/com/jetbrains/python/uv/
  sdk/configuration/uvVenv.kt; the project's SDK and the SDK table of
  jps/model-serialization/src/org/jetbrains/jps/model/serialization/
  JpsProjectConfigurationLoading.java, library/JpsSdkTableSerializer.java
  and module/JpsModuleRootModelSerializer.java; list popups of platform/
  platform-impl/src/com/intellij/ui/popup/list/ListPopupImpl.java,
  ListPopupModel.java, PopupListElementRenderer.java,
  PopupInlineActionsSupportImpl.kt and InlineActionsUtil.kt (row buttons,
  More Actions), ui/popup/WizardPopup.java (submenu placement), the
  submenu aim of platform/util/ui/src/com/intellij/ui/
  MouseMovementTracker.java and ScreenUtil.java (isMovementTowards), the
  run widget's spacing and stroke icons of platform/execution-impl/src/com/
  intellij/execution/ui/RedesignedRunWidget.kt and platform/platform-impl/
  src/com/intellij/openapi/wm/impl/headertoolbar/MainToolbar.kt, ui/popup/
  AbstractPopup.java and ActionPopupStep.java, platform/platform-api/src/
  com/intellij/ui/speedSearch/SpeedSearch.java and ui/
  GroupHeaderSeparator.java, platform/platform-api/src/com/intellij/
  openapi/ui/popup/util/PopupUtil.java, platform/util/ui/src/com/intellij/
  ui/TitlePanel.java and util/ui/JBUI.java (metrics); the desaturation of
  platform/core-ui/src/util/IconUtil.kt; the `.gitignore` of plugins/
  git4idea/backend/src/ignore/GitIgnoreInStoreDirGenerator.kt and
  platform/vcs-impl/src/com/intellij/openapi/vcs/changes/
  ChangeListManagerImpl.java; the UI strings of python/pluginResources/
  messages/PyBundle.properties. Only behaviour, numbers, formats and
  strings are used; reimplemented in Rust, no code is copied.
