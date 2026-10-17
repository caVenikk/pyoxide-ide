- crates/platform/workbench/src/layout.rs, the area sizes and splitter drags
  in src/workbench/tool_window_areas.rs, the stripe groups in src/chrome.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  layout and divider clamp of platform/platform-api/src/com/intellij/
  openapi/ui/ThreeComponentsSplitter.kt (doLayout, Divider.clamp), their use
  in platform/platform-impl/src/com/intellij/toolWindow/ToolWindowPane.kt
  (updateInnerMinSize) with the value of ide.mainSplitter.min.size in
  platform/util/resources/misc/registry.properties, the frame's minimum
  width of platform/platform-impl/src/com/intellij/openapi/project/impl/
  IdeProjectFrameAllocator.kt, and the stripe groups of platform/
  platform-impl/src/com/intellij/toolWindow/ToolWindowToolbar.kt. Changes:
  reimplemented in Rust on floating-point sizes; the gaps between the
  islands are the dividers and the minimum applies to the islands without
  them.
