- crates/platform/workbench/src/workbench/splits.rs (Move to Opposite Group,
  Open in Opposite Group, Open in Right Split, unsplit without duplicates,
  the Close actions of a group)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: EditorWindow.kt (unsplit, unsplitAll), EditorsSplitters.kt
  (openInRightSplit) and FileEditorManagerImpl.kt (getWindowToOpen,
  findWindowInAllSplitters) of platform/platform-impl/src/com/intellij/
  openapi/fileEditor/impl/, MoveEditorToOppositeTabGroupAction.kt there,
  OpenInRightSplitAction.kt, CloseAllEditorsAction.java and
  CloseEditorsActionBase.java of platform/platform-impl/src/com/intellij/
  ide/actions/, the texts of platform/platform-api/resources/messages/
  IdeBundle.properties. Behaviour and texts only, reimplemented in Rust, no
  code is copied.
