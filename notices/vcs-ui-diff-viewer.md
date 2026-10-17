- crates/platform/vcs-ui/src/diff/, crates/platform/editor/src/side_by_side.rs,
  crates/platform/workbench/src/workbench/diff.rs,
  crates/platform/workbench/src/tab_view.rs, the diff bindings of
  crates/platform/workbench/keymaps/macos.toml and default.toml,
  crates/platform/ui/assets/icons/diff/sideBySide.svg, unified.svg and
  crates/platform/ui/assets/icons/general/edit.svg (with their `_dark.svg`
  variants)
  The side-by-side diff viewer follows the IntelliJ Platform Community
  (Apache-2.0, license text in LICENSE-APACHE),
  https://github.com/JetBrains/intellij-community @ tag pycharm/2026.2.3:
  platform/diff-impl/src/com/intellij/diff/tools/simple/ (SimpleDiffViewer.java,
  SimpleDiffChangeUi.java: changed lines and words, Next and Previous
  Difference, the status text), platform/diff-impl/src/com/intellij/diff/
  tools/util/ (SyncScrollSupport.java, BaseSyncScrollable.java:
  synchronized scrolling and the scroll to a change;
  PrevNextDifferenceIterableBase.java; DiffSplitter.java and
  side/TwosideContentPanel.java, side/TwosideTextDiffViewer.java: the
  layout and the mirrored left side), platform/diff-impl/src/com/intellij/
  diff/util/ (DiffDividerDrawUtil.java, DiffDrawUtil.java: the divider's
  bands and curves, the empty ranges' lines, the layers;
  TextDiffTypeFactory.java: the faded colour of lines with changed words),
  platform/diff-impl/src/com/intellij/diff/impl/DiffEditorTitleDetails.kt
  and platform/vcs-impl/src/com/intellij/openapi/diff/impl/
  DiffTitleWithDetailsCustomizers.kt (the titles), the registry's
  `diff.divider.width`, the strings of platform/diff-api/resources/messages/
  DiffBundle.properties and the action texts of platform/platform-resources-en/
  src/messages/ActionsBundle.properties, the keys of platform/
  platform-resources/src/keymaps/ ($default.xml, Mac OS X 10.5+.xml).
  Behaviour, numbers, keys and texts reimplemented in Rust with GPUI; the
  line diff is git's histogram algorithm (imara-diff) instead of the
  reference's own, no unified viewer, no collapsed unchanged fragments,
  no per-change check boxes or apply arrows. The icons are unchanged
  copies of platform/icons/src/expui/diff/sideBySide.svg, unified.svg and
  platform/icons/src/expui/general/edit.svg (with the _dark variants).
