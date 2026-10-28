- crates/platform/ui/src/expandable.rs, the project tree's
  crates/platform/workbench/src/project_panel/row_expansion.rs and the
  object tree of crates/database/db-ui/src/explorer/tree_view.rs (the rest
  of a cut row shown over a tree's edge, a tree's highlight geometry and
  its sideways scrolling)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: platform/platform-impl/src/com/intellij/ui/
  AbstractExpandableItemsHandler.java and TreeExpandableItemsHandler.java
  (when the rest of an item shows, its place, background, border and
  rounded end, the update delay, what keeps it on screen), platform/
  platform-impl/src/com/intellij/ui/tree/ui/DefaultTreeUI.java (the highlight's
  extent: the inset from the view's edges, the control's position, a wide
  row's highlight) and platform/platform-impl/src/com/intellij/ui/render/
  RenderingHelper.java (a wide row's highlight is not shrunk to the view).
  Only behaviour and numbers are used; reimplemented in Rust, no code is
  copied.
