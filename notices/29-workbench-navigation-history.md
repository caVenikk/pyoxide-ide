- crates/platform/workbench/src/navigation.rs, src/workbench/history.rs,
  src/keymap.rs (mouse buttons), crates/platform/editor/src/editor.rs and
  find.rs (navigation jumps, the place's position in the view),
  crates/platform/code-intel-ui/src/navigation.rs (jumps in the file)
  The navigation history (Back and Forward) follows the IntelliJ Platform
  Community (Apache-2.0), https://github.com/JetBrains/intellij-community @
  pycharm/2026.2.3 (404a109fe630a2b9013f10b71081f87c9dbb32e2): the back and
  forward places of platform/platform-impl/src/com/intellij/openapi/
  fileEditor/impl/IdeDocumentHistoryImpl.kt (a place replaces the last one
  when it is the same place, Forward skips the current place, places in
  removed files are dropped), the same-place distance of .../fileEditor/
  impl/text/TextEditorState.java (4 lines) and the relative caret position
  of .../text/TextEditorProvider.kt and .../editor/ex/util/EditorUtil.java,
  the limit of platform/util/resources/misc/registry.properties
  (editor.navigation.history.stack.size, 150), the tab selection as a
  navigation of .../fileEditor/impl/EditorTabbedContainer.kt with the tab
  selected on closing of EditorWindow.kt and platform/platform-api/src/com/
  intellij/ui/tabs/impl/JBTabsImpl.kt, the caret moves that are
  navigations (the callers of includeCurrentCommandAsNavigation:
  .../editor/impl/EditorImpl.java, .../editor/actions/EditorActionUtil.java,
  .../ide/actions/GotoLineAction.java, platform/lang-impl/src/com/intellij/
  codeInsight/daemon/impl/GotoNextErrorHandler.java, .../ide/actions/
  SearchAgainAction.java, .../find/FindUtil.java, .../codeInsight/
  navigation/actions/ide.kt), and the keystrokes and mouse buttons of Back
  and Forward in the keymap files. Only behaviour and numbers are used;
  reimplemented in Rust, no code is copied.
