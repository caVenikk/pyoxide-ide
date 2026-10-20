- crates/platform/vcs-ui/src/diff/, crates/platform/editor/src/side_by_side.rs,
  crates/platform/editor/src/gutter.rs (the diff gutter), the icons
  crates/platform/ui/assets/icons/diff/gutterCheckBox@14x14.svg,
  gutterCheckBoxSelected@14x14.svg, gutterCheckBoxIndeterminate@14x14.svg and
  arrowRight@14x14.svg (with their `_dark.svg` variants)
  The diff viewer's look follows the IntelliJ Platform Community (Apache-2.0,
  license text in LICENSE-APACHE), https://github.com/JetBrains/intellij-community
  @ tag pycharm/2026.2.3: platform/diff-impl/src/com/intellij/diff/tools/simple/
  SimpleOnesideDiffViewer.java and tools/util/side/OnesideTextDiffViewer.java
  (one side for an added or a deleted file), util/DiffDrawUtil.java and
  util/DiffLineMarkerRenderer.kt (the gutter's paint modes, the lines of an
  excluded change), util/DiffDividerDrawUtil.java (`ExcludablePainter`),
  impl/ui/DiffHeaderToolbarPanel.kt (the toolbar's island),
  platform/platform-impl/src/com/intellij/openapi/editor/impl/
  EditorGutterComponentImpl.java (the separator line before the text, the
  whitespace separator), platform/vcs-impl/src/com/intellij/openapi/vcs/
  changes/actions/diff/lst/SimpleLocalChangeListDiffViewer.kt and
  LocalTrackerDiffUtil.kt (the status text with the included changes, the
  title's check box) and the strings of platform/diff-api/resources/messages/
  DiffBundle.properties. Behaviour, numbers and texts reimplemented in Rust
  with GPUI; the gutter's sizes are measured on the reference. The icons are
  unchanged copies of platform/icons/src/expui/diff/gutterCheckBox@14x14.svg,
  gutterCheckBoxSelected@14x14.svg, gutterCheckBoxIndeterminate@14x14.svg and
  arrowRight@14x14.svg (with the _dark variants).
