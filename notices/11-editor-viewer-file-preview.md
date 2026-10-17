- crates/platform/editor/src/viewer.rs (the read-only hint),
  crates/platform/workbench/src/file_preview.rs (the Soft-Wrap button's
  place, a found usage's marks),
  crates/platform/workbench/src/search_everywhere/ (text rows, the
  preview's occurrences)
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  information hint balloon of platform/platform-impl/src/com/intellij/
  codeInsight/hint/HintManagerImpl.java and HintUtil.java, ui/HintHint.java,
  ui/LightweightHint.java and ide/IdeTooltipManager.kt (padding, rounding,
  pointer size and place, callout shift); the floating toolbar's place of
  openapi/editor/impl/EditorImpl.java, openapi/editor/toolbar/floating/
  EditorFloatingToolbar.kt and AbstractFloatingToolbarComponent.kt, with the
  toolbar insets of platform/util/ui/src/com/intellij/util/ui/JBUI.java; the
  marks and caret of platform/usageView-impl/src/com/intellij/usages/impl/
  UsagePreviewPanel.kt; the rows of platform/lang-impl/src/com/intellij/
  find/impl/TextSearchListAgnosticRenderer.java and
  FileAndLineTextRenderer.java; the limit of highlighted matches of
  find/impl/livePreview/LivePreviewController. Reimplemented in Rust;
  numbers and behaviour only, no code is copied.
