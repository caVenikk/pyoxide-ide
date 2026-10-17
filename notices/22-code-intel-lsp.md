- crates/platform/code-intel, crates/platform/lsp-client,
  crates/platform/code-intel-ui, crates/python/py-lsp,
  docs/adr/0010-language-services.md, docs/adr/0011-type-server.md
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the severity mapping, tags
  and message format of platform/lsp/src/api/customization/
  LspDiagnosticsCustomizer.kt, the completion item rendering and kind icons
  of LspCompletionCustomizer.kt, the severity icons of platform/
  analysis-api/src/com/intellij/codeHighlighting/HighlightDisplayLevel.kt,
  the Language Services widget states of platform/lsp/src/api/lsWidget/
  LspClientWidgetItem.kt, the tool lookup of python/python-pytools/src/com/
  intellij/python/pytools/PyToolExt.kt and configuration/
  ExecutableDiscoveryMode.kt, the server command lines of python/
  python-ruff/src/com/intellij/python/ruff/server/RuffLspClient.kt, python/
  python-ty/src/com/intellij/python/ty/TyLspClient.kt and python/
  python-pyright/src/com/intellij/python/pyright/PyrightLspClient.kt, and
  the command titles from the action texts of platform/
  platform-resources-en/src/messages/ActionsBundle.properties. Only
  behaviour, formats and strings are used; reimplemented in Rust, no code
  is copied.
