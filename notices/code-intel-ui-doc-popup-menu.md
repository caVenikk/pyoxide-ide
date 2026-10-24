- crates/platform/code-intel-ui/src/doc.rs (the more menu, DocHistory),
  font_size.rs, doc_window.rs and crates/platform/settings/src/
  documentation.rs
  Adapted from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the rows and order of the
  documentation popup's more menu (platform/lang-impl/src/com/intellij/lang/
  documentation/ide/ui/DocumentationPopupUI.kt, the Documentation.* actions
  of platform/platform-resources/src/idea/LangActions.xml), the history of
  impl/DocumentationBrowserHistory.kt, the options of
  actions/ToggleShowDocsInPopupAction.kt and codeInsight/documentation/
  ToggleShowDocsOnHoverAction.java, the sizes of platform/editor-ui-api/src/
  com/intellij/openapi/options/FontSize.java and codeInsight/documentation/
  DocumentationFontSize.kt, the popup of platform/lang-impl/src/com/intellij/
  ui/FontSizePopup.kt with the slider metrics and colours of
  platform/platform-impl/src/com/intellij/ide/ui/laf/darcula/ui/
  DarculaSliderUI.kt, the tool window of impl/DocumentationToolWindowManager.kt,
  and the texts of CodeInsightBundle.properties, ActionsBundle.properties,
  ApplicationBundle.properties and IdeBundle.properties. Changes: rewritten in
  Rust on GPUI; the three options are one settings table; the tool window
  shows the last documentation without tabs, auto-update or its own toolbar.
