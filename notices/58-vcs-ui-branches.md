- crates/platform/vcs-ui/src/ (widget.rs, branches.rs, new_branch.rs),
  crates/platform/vcs/src/branch_name.rs, crates/platform/git/src/branches.rs,
  the locale of crates/platform/git/src/exe.rs,
  crates/platform/workbench/src/workbench/branches.rs, the VCS widget of
  crates/platform/workbench/src/chrome.rs, the branch keys of
  crates/platform/workbench/keymaps/macos.toml and default.toml,
  crates/platform/ui/assets/icons/general/vcs.svg, nodes/star.svg,
  vcs/{fetch,push,update,commit,branchLabel,currentBranchLabel,
  currentBranchFavoriteLabel,changesPush@12x12,changesUpdate@12x12}.svg
  (and _dark)
  The VCS widget, the branches popup and the branch dialogs follow the
  IntelliJ Platform Community (Apache-2.0, license text in LICENSE-APACHE),
  https://github.com/JetBrains/intellij-community @ tag pycharm/2026.2.3:
  plugins/git4idea/frontend/src/com/intellij/vcs/git/frontend/widget/
  GitToolbarWidgetAction.kt, plugins/git4idea/backend/src/ui/branch/
  GitCurrentBranchPresenter.kt, branch/GitBranchUtil.java (the widget's
  text, icon and tooltip), plugins/git4idea/shared/src/com/intellij/vcs/git/
  ui/GitBranchPresentation.java (names cut in the middle beyond 80
  characters), GitIncomingOutgoingUi.kt and branch/GitInOutState.kt (arrows,
  colours, tooltips), branch/popup/ and branch/tree/ (GitBranchesPopupBase,
  GitDefaultBranchesPopupStep, GitDefaultBranchesTreeRenderer,
  GitBranchesTreeModel, GitBranchesTreeSingleRepoModel,
  GitBranchesTreeModelUtil: groups, folders, order, preferred selection,
  sizes), plugins/git4idea/backend/src/branch/GitNewBranchDialog.kt,
  validators/GitRefNameValidator.java and GitBranchValidator.kt (names
  cleaned while typing and checked), branch/GitSmartOperationDialog.java,
  repo/GitRecentCheckoutBranches.kt, actions/GitInit.java and the branch
  actions of actions/branch/ and actions/ref/,
  platform/vcs-impl/src/com/intellij/vcs/VcsLocaleHelper.kt (git's messages
  in English), the texts of plugins/git4idea/shared/resources/messages/
  GitBundle.properties and platform/dvcs-impl/shared/resources/messages/
  DvcsBundle.properties, the action groups of plugins/git4idea/backend/
  resources/intellij.vcs.git.backend.xml and the keys of
  platform/platform-resources/src/keymaps/. Only behaviour, numbers, keys and
  texts are used; reimplemented in Rust. The icons are unchanged copies of
  platform/icons/src/expui/general/vcs.svg, nodes/star.svg,
  vcs/{fetch,push,update,commit}.svg and platform/dvcs-impl/shared/
  resources/icons/new/{branchLabel,currentBranchLabel,
  currentBranchFavoriteLabel,changesPush@12x12,changesUpdate@12x12}.svg
  (with the _dark variants) @ tag pycharm/2026.2.3.
