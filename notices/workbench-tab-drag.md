- crates/platform/workbench/src/workbench/splits/tab_drag.rs (dragging an
  editor tab: the drop side of an editor, the slot in a tab strip, the drop
  area, moving or copying the file, the pinned state of a dropped tab) and
  the drag area colour `ui.drag-and-drop-area` of the themes crate
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: TabsUtil.java (getDropSideFor with the ratio of the
  registry key ide.tabbedPane.dragToSplitRatio, updateBoundsWithDropSide) of
  platform/platform-api/src/com/intellij/ui/tabs/, JBTabsImpl.kt
  (processDropOver, startDropOver) and TabLabel.kt (the drop target tab) of
  platform/platform-api/src/com/intellij/ui/tabs/impl/, DragHelper.java,
  EditorTabbedContainer.kt (EditorTabbedContainerDragOutDelegate: Control
  copies) and DockableEditorTabbedContainer.kt (add, MyDropAreaPainter) of
  platform/platform-impl/src/com/intellij/openapi/fileEditor/impl/, and the
  colour DragAndDrop.areaBackground of JBUI.java and the theme files.
  Only behaviour and values are used; reimplemented in Rust, no code is
  copied.
