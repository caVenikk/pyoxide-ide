- crates/platform/vcs-ui/src/marker_popup.rs (the toolbar row with the
  Changelists group and the commit field, the old text as a read-only editor,
  the word differences), crates/platform/editor/src/diff_highlights.rs and
  its painting in element.rs, the reference default of
  ui.marker-popup-border in crates/platform/themes/src/import/convert.rs,
  crates/platform/ui/assets/icons/vcs/changelist.svg, general/dropdown.svg,
  vcs/amendInline.svg and vcs/commitInline.svg (and _dark)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: platform/diff-impl/src/com/intellij/openapi/vcs/ex/
  LineStatusMarkerPopupPanel.java (the text field of the old text, its
  preferred size, the word highlighters of the popup and of the editor, the
  border colour's default), LineStatusTrackerMarkerRenderer.kt (word
  differences of modified ranges only), platform/vcs-impl/src/com/intellij/
  openapi/vcs/ex/LineStatusTracker.kt and PartialLocalLineStatusTracker.kt
  (the Changelists group and the commit panel in the toolbar row),
  ex/commit/CommitChunkComponent.kt and vcs/commit/CommitInputBorder.kt (the
  commit panel's width, gap, button size and insets),
  platform/diff-impl/src/com/intellij/diff/util/DiffEmptyHighlighterRenderer.java
  (the bar of an empty word) and TextDiffTypeFactory.java (the colour of the
  changed lines), platform/platform-impl/src/com/intellij/openapi/
  actionSystem/ex/ActionButtonLook.java (the corner arrow of a popup group),
  the strings ex.changelists, commit.from.gutter.placeholder and
  checkbox.amend of platform/vcs-api/vcs-api-core/resources/messages/
  VcsBundle.properties. Only behaviour, numbers and texts are used;
  reimplemented in Rust; the word differences are a word-level Myers diff,
  not a copy of ByWord. The icons are unchanged copies of
  platform/icons/src/expui/vcs/changelist.svg, platform/icons/src/expui/
  general/dropdown.svg and platform/vcs-impl/resources/icons/AmendInline.svg
  and CommitInline.svg (with the _dark variants), renamed to lower camel
  case.
