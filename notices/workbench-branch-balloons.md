- crates/platform/workbench/src/workbench/branches.rs (the texts of the
  branch balloons), the bold and code parts of balloon content in
  crates/platform/ui/src/notification.rs
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: the success messages of plugins/git4idea/backend/src/
  branch/GitDeleteBranchOperation.java, GitCheckoutOperation.java,
  GitCreateBranchOperation.kt and GitRenameBranchOperation.java with the
  strings delete.branch.operation.deleted.branch.bold,
  checkout.operation.checked.out, checkout.operation.checked.out.new.branch.from,
  create.branch.operation.branch.created and git.rename.branch.was.renamed.to
  of plugins/git4idea/shared/resources/messages/GitBundle.properties, the
  names wrapped by plugins/git4idea/backend/src/util/GitUIUtil.java (code),
  and the code style of platform/platform-impl/src/com/intellij/ide/ui/html/
  LafCssProvider.kt and GlobalStyleSheetHolder.kt (the editor font for
  code in HTML content). Only texts and behaviour are used; reimplemented in
  Rust as a small subset of HTML (b, code, br and the common entities).
