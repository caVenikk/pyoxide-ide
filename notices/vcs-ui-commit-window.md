- crates/platform/vcs-ui/src/commit/, crates/platform/vcs/src/commit.rs,
  crates/platform/git/src/commit.rs,
  crates/platform/workbench/src/workbench/commit.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  non-modal commit panel of platform/vcs-impl/src/com/intellij/vcs/commit/
  (NonModalCommitPanel.kt, ChangesViewCommitPanel.kt, CommitActionsPanel.kt,
  CommitStatusPanel.kt, CommitProgressPanel.kt, CommitOptionsPanel.kt,
  AmendCommitHandlerImpl.kt, AmendCommitModeDropDownLink.kt,
  ToggleAmendCommitModeAction.kt, ShowNotificationCommitResultHandler.kt:
  the layout and insets, the errors, Amend and its message, the commit
  options popup, the balloon), the legend and the tree of platform/vcs-impl/
  src/com/intellij/openapi/vcs/changes/ui/ (CommitLegendPanel.kt,
  ChangeInfoCalculator.kt, RollbackChangesDialog.kt) and platform/vcs-impl/
  shared/src/com/intellij/openapi/vcs/changes/ui/ (ChangesTree.java,
  ChangesBrowserNode.java, ChangesBrowserChangeNode.java,
  TreeModelBuilder.java: inclusion toggling, counts, the parent folder and
  "renamed from");
  the commit, rollback and user name handling of plugins/git4idea/backend/
  src/checkin/ (GitCommitOptions.kt, GitSkipHooksCommitHandlerFactory.kt,
  GitUserNameNotDefinedDialog.kt, GitCheckinHandlerFactory.kt) and
  plugins/git4idea/backend/src/rollback/GitRollbackEnvironment.java; the
  toolbar and context menu groups of platform/vcs-impl/resources/META-INF/
  VcsActions.xml; strings of VcsBundle.properties, GitBundle.properties,
  DvcsBundle.properties, DiffBundle.properties and ActionsBundle.properties.
  Changes: reimplemented in Rust with GPUI; one repository, no change
  lists, no grouping by folder and no commit checks other than the user
  name, detached HEAD and Git hooks.
