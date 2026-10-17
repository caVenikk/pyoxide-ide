- crates/platform/vcs-ui/src/ (status_colors.rs, markers.rs, marker_popup.rs),
  crates/platform/workbench/src/workbench/version_control/markers.rs,
  crates/platform/workbench/keymaps/macos.toml and default.toml (Next and
  Previous Change, Rollback Lines, Show Diff for Lines in the marker popup),
  the version control roles of crates/platform/themes,
  crates/platform/ui/assets/icons/vcs/revert.svg, diff.svg (and _dark)
  File status colours, change markers and the marker popup follow the IntelliJ
  Platform Community (Apache-2.0, license text in LICENSE-APACHE),
  https://github.com/JetBrains/intellij-community @ tag pycharm/2026.2.3:
  platform/diff-impl/src/com/intellij/openapi/vcs/ex/
  (LineStatusTrackerBlockOperations.kt: next and previous range, ranges of
  lines; RangesBuilder.kt: whitespace-only inner ranges;
  LineStatusMarkerPopupPanel.java: the popup's layout, borders and insets;
  LineStatusMarkerPopupActions.kt, LineStatusMarkerRendererWithPopup.kt and
  LineStatusMarkerPopupService.kt: the actions, placement and closing;
  LineStatusMarkerRenderer.kt: thin error stripe marks),
  platform/diff-impl/src/com/intellij/openapi/diff/LineStatusMarkerColorScheme.kt
  (the colour keys), platform/vcs-impl/src/com/intellij/openapi/vcs/ex/
  LineStatusTracker.kt and RollbackLineStatusAction.java (the popup's actions,
  Rollback Lines), the action texts of platform/vcs-impl/resources/META-INF/
  VcsActions.xml and the keys of platform/platform-resources/src/keymaps/
  ($default.xml, Mac OS X.xml). Only behaviour, numbers, keys and texts are
  used; reimplemented in Rust. The icons are unchanged copies of
  platform/icons/src/expui/vcs/revert.svg and diff.svg (with the _dark
  variants) @ f5a0bddb9cee9135c700de9a67534e11cdf61c07.
