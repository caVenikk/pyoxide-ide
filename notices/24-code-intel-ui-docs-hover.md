- crates/platform/ui/src/markdown.rs, crates/platform/code-intel-ui/src/
  doc.rs, hover.rs, signature.rs, actions.rs, navigation.rs, list_popup.rs,
  intentions.rs, format.rs
  Follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the tooltip delay of
  platform/ide-core-impl/src/com/intellij/openapi/editor/ex/
  EditorSettingsExternalizable.java (500 ms), the parameter info delay of
  platform/analysis-impl/src/com/intellij/codeInsight/CodeInsightSettings.java
  (1000 ms), the problem text of platform/lsp/src/api/customization/
  LspDiagnosticsCustomizer.kt (message, a non-breaking space, the source
  and the rule as a link), the keystrokes of QuickJavaDoc, ParameterInfo,
  ShowErrorDescription, GotoDeclaration, ShowUsages, ShowIntentionActions,
  ReformatCode and OptimizeImports in the keymap files, and the texts
  "Cannot find declaration to go to" and "Choose Declaration" as the
  reference shows them; and @
  2bf84a9b80f1fb7dc2ca7dd23aefa96ef4a16643 (master): the documentation
  layout of platform/lsp-impl/src/impl/features/documentation/
  LspDocumentationData.kt (a leading fenced code block is the declaration,
  a rule right after it is left out), the parameter ranges and current
  parameter of .../features/parameterInfo/LspParameterInfoHandler.kt, and
  the names-only parameter list and the hidden overloads of python/src/com/
  jetbrains/python/PyParameterInfoHandler.java with their links "Show {0}
  more overloads" and "Show less" (python/pluginResources/messages/
  PyBundle.properties). The list popup of navigation and intention actions
  follows ui::menu (see its entry above). Only behaviour, numbers, formats
  and strings are used; reimplemented in Rust, no code is copied.
