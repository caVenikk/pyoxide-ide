- crates/platform/code-intel-ui (completion and the Language Services
  widget), crates/platform/ui/src/keys.rs (macOS key symbols)
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the lookup keys of platform/
  lang-impl/src/com/intellij/codeInsight/lookup/impl/LookupActionHandler.java
  and actions/ChooseItemAction.java, the row layout and match colour of
  LookupCellRenderer.kt, the hint "Press {0} to insert, {1} to replace"
  (completion.ad.press.0.to.insert.1.to.replace of platform/analysis-api/
  resources/messages/CodeInsightBundle.properties) and the focus of
  automatic popups of platform/lang-impl/src/com/intellij/codeInsight/
  completion/CompletionProgressIndicator.java, the matching and its
  leading-underscore rules of platform/analysis-impl/src/com/intellij/
  codeInsight/completion/impl/CamelHumpMatcher.java, the order of
  BaseCompletionService.defaultSorter and platform/lsp-impl/src/impl/
  features/completion/LspCompletionWeigher.kt, the automatic popup of
  platform/lang-impl/src/com/intellij/codeInsight/editorActions/
  CompletionAutoPopupHandler.java and LspAutoPopupTypedHandler.kt, the
  parentheses of platform/analysis-api/src/com/intellij/codeInsight/
  completion/util/ParenthesesInsertHandler.java and python/python-psi-impl/
  src/com/jetbrains/python/codeInsight/completion/PyFunctionInsertHandler.java,
  the widget and popup of platform/lang-impl/src/com/intellij/platform/lang/
  lsWidget/impl/LanguageServiceWidget.kt, platform/lsp/src/api/lsWidget/
  LspClientWidgetItem.kt and platform/lsp-impl/src/impl/lsWidget/
  LspWidgetInternalServiceImpl.kt with their strings in platform/lang-api/
  resources/messages/LangBundle.properties and platform/lsp/resources/
  messages/LspBundle.properties, and the macOS key symbols of platform/
  platform-api/src/com/intellij/openapi/keymap/MacKeymapUtil.java. Only
  behaviour and strings are used; reimplemented in Rust, no code is copied.
