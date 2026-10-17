- crates/platform/project/src/filter.rs, store.rs, idea.rs,
  crates/platform/ui/src/menu.rs, crates/platform/workbench/src/project_panel.rs
  (context menu), docs/adr/0008-project-folder-and-exclusions.md
  The project model follows the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  0b33b8b8e90353a31a9db5b8fcecc6626d94967e: the default list of ignored
  files and folders of platform/platform-impl/src/com/intellij/openapi/
  fileTypes/impl/FileTypeManagerImpl.java and the mask rules of
  jps/model-impl/src/org/jetbrains/jps/model/fileTypes/impl/
  FileNameMatcherFactoryImpl.java, jps/model-impl/src/com/intellij/openapi/
  fileTypes/impl/FileTypeAssocTable.java and WildcardFileNameMatcher.java;
  the module file format read by jps/model-serialization/src/org/jetbrains/
  jps/model/serialization/JpsProjectLoader.java, module/
  JpsModuleRootModelSerializer.java and PathMacroUtil.java; the exclusion
  behaviour of platform/projectModel-impl/src/com/intellij/workspaceModel/
  core/fileIndex/impl/workspaceFileSets.kt, platform/lang-impl/src/com/
  intellij/psi/impl/file/SourceRootIconProvider.java, .../find/impl/
  FindInProjectTask.java and FileScanner.kt, platform/core-api/src/com/
  intellij/openapi/project/ProjectCoreUtil.kt, .../ide/projectView/actions/
  MarkExcludeRootAction.java and UnmarkRootAction.java, and the virtual
  environment exclusion of python/python-pyproject/src/com/intellij/python/
  pyproject/model/internal/platformBridge/VenvExcluder.kt and
  python/src/com/jetbrains/python/sdk/PySdkExt.kt. The context menu
  follows the popup menu metrics of platform/util/ui/src/com/intellij/util/
  ui/JBUI.java (PopupMenu.Selection insets and arc, List.rowHeight),
  platform/platform-impl/src/com/intellij/ui/plaf/beg/IdeaMenuUI.java and
  BegMenuItemUI.java (item height and widths), .../ide/ui/laf/darcula/ui/
  DarculaPopupMenuBorder.java and DarculaMenuSeparatorUI.java,
  .../ui/WindowRoundedCornersManager.java (the popup border colour),
  platform/platform-resources/src/themes/darcula.theme.json and
  intellijlaf.theme.json (PopupMenu, PopupMenuSeparator) and the Islands
  theme files (Popup.borderColor, Menu.borderColor, Ide.Shadow), the menu
  structure of platform/platform-impl/resources/idea/LangActions.xml and
  python/resources/META-INF/pycharm-core.xml, and the UI strings "Mark
  Directory As", "Mark File As", "Excluded" and "Cancel Exclusion" of
  platform/platform-resources-en/src/messages/ActionsBundle.properties and
  platform/lang-api/resources/messages/LangBundle.properties.
  Only the list of names, the file format, numbers and strings are used;
  reimplemented in Rust, no code is copied.
