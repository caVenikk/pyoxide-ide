- crates/platform/workbench/src/workbench.rs (file drops, empty editor area)
  The handling of files dropped onto the main window follows the IntelliJ
  Platform Community (Apache-2.0), https://github.com/JetBrains/intellij-community
  @ 27e7bdec710a61fe64b065c45585156355a1c686: the drop targets of
  platform/platform-impl/src/com/intellij/openapi/fileEditor/impl/
  EditorsSplitters.kt and EditorTabbedContainer.kt, the order of opening and
  the folders of .../openapi/editor/FileDropManager.kt, .../fileEditor/
  FileNavigatorImpl.kt and ProjectFileNavigatorImpl.kt, the welcome
  screen's drop of .../wm/impl/welcomeScreen/ProjectsTabFactory.kt and
  .../ide/impl/ProjectUtil.kt (openOrImportFilesAsync), and the drop target
  of platform/lang-impl/src/com/intellij/ide/projectView/impl/
  ProjectViewDropTarget.java. The empty editor area follows .../fileEditor/
  impl/DefaultEditorEmptyTextProvider.kt (the list and order of the hints)
  and EditorEmptyTextPainter.java with platform/util/ui/src/com/intellij/
  util/ui/UIUtil.java (TextPainter: font size, line spacing, position; the
  shortcut colour `Shortcut.foreground`, the `ui.shortcut` theme role), with
  the UI strings "Project View" and "Drop files here to open them" of
  platform/platform-api/resources/messages/IdeBundle.properties. Only
  behaviour, numbers and strings are used; reimplemented in Rust, no code is
  copied.
