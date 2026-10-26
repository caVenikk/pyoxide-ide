- crates/platform/workbench/src/workbench/tab_menu.rs, the tab drawing in
  workbench.rs and the mnemonics of crates/platform/ui/src/menu.rs (the
  context menu of an editor tab, its Close actions, pinned tabs)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: the groups EditorTabPopupMenu, EditorTabPopupMenuEx and
  CloseEditorsGroup of platform/platform-resources/src/idea/
  PlatformActions.xml and LangActions.xml, the texts and mnemonics of
  platform/platform-resources-en/src/messages/ActionsBundle.properties and
  platform/platform-api/resources/messages/IdeBundle.properties, the rules
  of platform/platform-impl/src/com/intellij/ide/actions/
  CloseEditorsActionBase.java, CloseAllEditorsButActiveAction.java,
  CloseAllEditorsAction.java, CloseAllUnpinnedEditorsAction.java,
  CloseAllEditorsToTheLeftAction.java, CloseAllEditorsToTheRightAction.java,
  PinActiveTabAction.java, RevealFileAction.java and platform/vcs-impl/src/
  com/intellij/ide/actions/CloseAllUnmodifiedEditorsAction.java, the pinned
  tab's icon of platform/platform-impl/src/com/intellij/openapi/fileEditor/
  impl/tabActions/CloseTab.kt, the grouping of pinned tabs of
  platform/platform-api/src/com/intellij/ui/tabs/impl/JBTabsImpl.kt
  (groupPinnedFirst) and the Git menu of plugins/git4idea/backend/resources/
  intellij.vcs.git.backend.xml. Only behaviour and texts are used;
  reimplemented in Rust, no code is copied.
