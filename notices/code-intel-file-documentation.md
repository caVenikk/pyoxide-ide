- crates/platform/code-intel-ui (src/file_doc.rs; the file documentation,
  the link row and the corner toolbar of src/doc.rs; Jump to Source and View
  External Documentation of src/hover.rs and src/commands.rs),
  crates/platform/ui (src/locale.rs; assets/icons/ide/externalLink.svg and
  externalLink_dark.svg), crates/platform/workbench/keymaps (the bindings of
  documentation.edit-source and documentation.view-external)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the file section of quick
  documentation of platform/lang-impl/src/com/intellij/codeInsight/
  documentation/DocumentationManager.java (generateFileDoc: VCS status,
  scope with its file colour, size, type, modification and creation time;
  getLink: "`{0}` on {1}" with the URL's host; the lines between sections of
  decorate) and platform/lang-impl/src/com/intellij/lang/documentation/psi/
  PsiElementDocumentationTarget.kt; the labels of platform/lang-api/
  resources/messages/CodeInsightBundle.properties; the scope of platform/
  lang-impl/src/com/intellij/psi/search/scope/NonProjectFilesScope.java with
  its default colour; the corner toolbar of platform/lang-impl/src/com/
  intellij/lang/documentation/ide/ui/DocumentationPopupUI.kt and ui.kt and
  the paddings of platform/lang-impl/src/com/intellij/codeInsight/
  documentation/DocumentationHtmlUtil.kt; the file size format of platform/
  util-rt/src/com/intellij/openapi/util/text/StringUtilRt.java
  (formatFileSize); the date format of platform/platform-api/src/com/
  intellij/util/text/DateFormatUtil.java (the system's short date and short
  time); the texts "Jump to Source" and "View External Documentation" of
  ActionsBundle.properties and the shortcuts of EditSource and
  ExternalJavaDoc in the keymap files; the icon platform/icons/src/expui/ide/
  externalLink.svg and externalLink_dark.svg (unmodified copies). Changes:
  reimplemented in Rust with GPUI; the scope covers Non-Project Files only;
  the more menu holds View External Documentation only; the corner toolbar
  shows on the documentation of a whole file only.
