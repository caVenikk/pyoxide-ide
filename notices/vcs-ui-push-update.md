- crates/platform/vcs-ui/src/ (push.rs, update.rs, operations.rs, the
  remote branch actions of branches.rs), crates/platform/vcs/src/remote.rs,
  crates/platform/git/src/remote.rs, crates/platform/workbench/src/
  workbench/remote.rs, the Push, Update Project and VCS Operations keys of
  crates/platform/workbench/keymaps/macos.toml and default.toml and their
  rows in actions.toml
  The Push dialog, push results and "Push Rejected", Force Push, Update
  Project, Fetch, Pull and the VCS Operations popup follow IntelliJ
  Platform Community (Apache-2.0), https://github.com/JetBrains/
  intellij-community @ tag pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2):
  platform/dvcs-impl/src/com/intellij/dvcs/push/ui/ (VcsPushDialog.java,
  PushLog.java, ForcePushAction.kt: the layout, sizes and the force push
  confirmation), plugins/git4idea/backend/src/push/ (GitPushOperation.java,
  GitPushResultNotification.java, GitRejectedPushUpdateDialog.kt,
  GitPushSupport.java, GitPushTarget.java, GitPushTargetPanel.java,
  GitPushNativeResultParser.java, GitOutgoingCommitsProvider.java: targets,
  outgoing commits, the porcelain results, when a rejected push asks to
  update), plugins/git4idea/backend/src/update/ (GitUpdateProcess.java,
  GitUpdateSession.kt, GitUpdateInfoAsLog.kt, GitUpdatedRanges.kt,
  GitUpdateOptionsPanel.kt, the merge and rebase updaters: the checks
  before an update, the stash around it, the counts),
  platform/vcs-impl/src/com/intellij/openapi/vcs/update/
  UpdateOptionsDialog.kt (the Update Project dialog),
  plugins/git4idea/backend/src/fetch/GitFetchSupportImpl.kt (pruned
  references, the balloons), plugins/git4idea/backend/src/ui/branch/
  GitBranchActionsUtil.kt and actions/branch/GitPullBranchAction.kt (a
  branch's Update, the pulls), platform/vcs-impl/src/com/intellij/openapi/
  vcs/actions/VcsQuickListPopupAction.java and plugins/git4idea/backend/
  src/actions/GitQuickListContentProvider.java (the VCS Operations popup),
  the texts of plugins/git4idea/shared/resources/messages/
  GitBundle.properties, platform/dvcs-impl/shared/resources/
  messages/DvcsBundle.properties and platform/vcs-api/vcs-api-core/
  resources/messages/VcsBundle.properties, the actions of
  platform/dvcs-impl/resources/intellij.platform.vcs.dvcs.impl.xml,
  platform/vcs-impl/resources/META-INF/VcsActions.xml and
  plugins/git4idea/backend/resources/intellij.vcs.git.backend.xml, and the
  keys of platform/platform-resources/src/keymaps/. Only behaviour,
  numbers, keys and texts are used; reimplemented in Rust with GPUI. The
  dialogs are drawn in the main window, the remembered choices last while
  the IDE runs, and git never asks for credentials.
