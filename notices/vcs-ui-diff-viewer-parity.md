- crates/platform/vcs-ui/src/diff/, crates/platform/editor/src/side_by_side.rs,
  crates/platform/editor/src/gutter.rs (the diff gutter and its action icons),
  crates/platform/vcs-ui/src/commit/tree.rs (changes left out of a file),
  crates/platform/vcs/src/ranges.rs (`partial_text`),
  crates/platform/git/src/commit.rs (the partial commit),
  crates/platform/git/src/idea.rs (the commit messages of `.idea/workspace.xml`),
  crates/platform/workbench/src/workbench/diff.rs (the files of all groups),
  crates/platform/ui/src/form.rs (the segmented button's look), the theme
  roles `segmented-selected` and `segmented-selected-border`, the icons
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
  DiffBundle.properties, tools/util/SyncScrollSupport.java (both sides moving
  to a change together), platform/platform-impl/src/com/intellij/openapi/
  editor/impl/ScrollingModelImpl.java (the duration of an animated scroll),
  platform/platform-api/src/com/intellij/util/animation/Easing.java
  (`EASE_OUT`), platform/platform-impl/src/com/intellij/ui/dsl/builder/
  components/SegmentedButtonComponent.kt (the selected choice's outline over
  the button's), tools/simple/SimpleDiffChangeUi.java and util/
  DiffGutterOperation.java (the `>>` of a change by the left numbers, "Revert",
  "Replace change"), LocalTrackerDiffUtil.kt's `createCheckboxToggle` and
  `getStatusText` and platform/vcs-impl/src/com/intellij/openapi/vcs/ex/
  PartialLocalLineStatusTracker.kt (a change's check box by the right numbers,
  the file partly included), plugins/git4idea/backend/src/checkin/
  GitCheckinEnvironment.kt (a partial commit staging the included content),
  platform/vcs-impl/src/com/intellij/openapi/vcs/changes/
  ChangeViewDiffRequestProcessor.java and ChangesViewDiffPreviewProcessor.kt
  (the next file across the groups, the counter of the file's group),
  platform/vcs-impl/src/com/intellij/vcs/commit/ChangesViewCommitMessagePolicy.kt
  and platform/vcs-api/shared/src/com/intellij/openapi/vcs/VcsConfiguration.java
  (where the unsent and the recent commit messages are kept), the
  `Diff.ApplyLeftSide` keystrokes of platform/platform-resources/src/keymaps/
  and the `SegmentedButton` colours of platform/
  platform-resources/src/themes/islands/ManyIslandsDark.theme.json and
  ManyIslandsLight.theme.json. Behaviour, numbers and texts reimplemented in Rust
  with GPUI; the gutter's sizes are measured on the reference. The icons are
  unchanged copies of platform/icons/src/expui/diff/gutterCheckBox@14x14.svg,
  gutterCheckBoxSelected@14x14.svg, gutterCheckBoxIndeterminate@14x14.svg and
  arrowRight@14x14.svg (with the _dark variants).
