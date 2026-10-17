- crates/platform/project/src/fs_ops.rs, crates/platform/workbench/src/
  project_panel/ (file_ops.rs, name_popup.rs, new_items.rs, transfer.rs,
  transfer_dialog.rs), crates/platform/workbench/src/workbench/
  file_events.rs, crates/python/pyoxide-ide/src/new_items.rs,
  crates/platform/ui/assets/icons/general/cut.svg, copy.svg, paste.svg
  The file operations of the project tool window follow the IntelliJ
  Platform Community (Apache-2.0), https://github.com/JetBrains/intellij-community
  @ 404a109fe630a2b9013f10b71081f87c9dbb32e2 (tag pycharm/2026.2.3): the
  checks of new names of platform/lang-impl/src/com/intellij/ide/actions/
  FileNameState.kt, CreateFileAction.java and
  CreateDirectoryOrPackageHandler.java; the popups of
  CreateDirectoryOrPackageAction.java and platform/platform-impl/src/com/
  intellij/ide/ui/newItemPopup/NewItemPopupUtil.java and
  NewItemSimplePopupPanel.java; the selection after creating of
  platform/lang-impl/src/com/intellij/ide/projectView/impl/
  IdeViewForProjectViewPane.java; the Rename dialog of platform/lang-impl/
  src/com/intellij/refactoring/rename/RenameDialog.java and
  RenamePsiFileProcessor.java; the Delete question of platform/lang-impl/
  src/com/intellij/ide/util/DeleteHandler.java and DeleteUtil.java; the
  trash rules of platform/platform-util-io/src/com/intellij/util/io/
  TrashBin.java and platform/ide-core/src/com/intellij/ide/
  GeneralSettings.kt; the order of the context menu of platform/
  platform-resources/src/idea/LangActions.xml (ProjectViewPopupMenu); the
  clipboard, copy and move of platform/lang-impl/src/com/intellij/ide/
  CopyPasteDelegator.java, .../refactoring/copy/CopyHandler.java,
  CopyFilesOrDirectoriesHandler.java and CopyFilesOrDirectoriesDialog.java,
  .../refactoring/move/MoveHandler.java and moveFilesOrDirectories/
  MoveFilesOrDirectoriesHandler.java, MoveFilesOrDirectoriesUtil.java and
  MoveFilesOrDirectoriesDialog.java, .../refactoring/SkipOverwriteChoice.java,
  platform/platform-impl/src/com/intellij/ide/actions/CopyPathsAction.java
  and platform/lang-impl/src/com/intellij/ide/actions/
  CopyReferenceAction.java; Python's New items of python/src/com/
  jetbrains/python/actions/CreatePythonFileAction.java and
  CreatePackageAction.java, with the text of the internal file template
  python/pluginResources/fileTemplates/internal/Python Unit Test.py.ft
  (copied unchanged) and the strings of python/pluginResources/messages/
  PyBundle.properties; Reload from Disk of platform/lang-impl/src/com/
  intellij/ide/actions/SynchronizeCurrentFileAction.java; the look of
  drag and drop of platform/platform-impl/src/com/intellij/ide/dnd/aware/
  DnDAwareTree.java (the dragged row's image), .../ide/dnd/Highlighters.java
  and platform/lang-impl/src/com/intellij/ide/projectView/impl/
  ProjectViewDropTarget.java (the drop target rectangle) with the colours
  DragAndDrop.borderColor and DragAndDrop.rowBackground of platform/util/
  ui/src/com/intellij/util/ui/JBUI.java (theme roles drag-and-drop-border
  and drag-and-drop-row); the icons platform/icons/src/expui/general/
  cut.svg, copy.svg, paste.svg and refresh.svg with their dark variants
  (unchanged copies); and
  the UI strings of platform/platform-api/resources/messages/
  IdeBundle.properties, platform/core-api/resources/messages/
  CoreBundle.properties, platform/lang-api/resources/messages/
  LangBundle.properties, platform/platform-resources-en/src/messages/
  ActionsBundle.properties and platform/refactoring/resources/messages/
  RefactoringBundle.properties. Only behaviour and strings are used;
  reimplemented in Rust, no code is copied. Moving to the trash uses the
  `trash` crate (MIT).
