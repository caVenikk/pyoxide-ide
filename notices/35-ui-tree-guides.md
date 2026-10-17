- crates/platform/ui/src/tree_guides.rs, crates/platform/workbench/src/
  project_panel/row_expansion.rs
  The rows of trees follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  404a109fe630a2b9013f10b71081f87c9dbb32e2 (tag pycharm/2026.2.3): the
  indent guides of platform/platform-impl/src/com/intellij/ui/tree/ui/
  ClassicPainter.java (a line at the centre of each ancestor's expand
  control, the colour Tree.hash, theme role tree-indent-guide) and the
  option showTreeIndentGuides of platform/editor-ui-api/src/com/intellij/
  ide/ui/UISettingsState.kt; the rest of a cut row shown over the tree's
  edge of platform/platform-impl/src/com/intellij/ui/
  TreeExpandableItemsHandler.java and AbstractExpandableItemsHandler.java
  (when it shows, its place, background and border). Only behaviour,
  numbers and colours are used; reimplemented in Rust, no code is copied.
