- crates/platform/code-intel-ui/src/completion/mod.rs and view.rs (the
  completion characters that insert the chosen item)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: the characters of platform/lang-impl/src/com/intellij/
  codeInsight/completion/DefaultCharFilter.java, the handling of a typed
  character of platform/lang-impl/src/com/intellij/codeInsight/lookup/impl/
  LookupTypedHandler.java (the chosen item goes in first, a name completed
  through the typed character, the focus degree of LookupImpl.java and
  CompletionProgressIndicator.java) and the completion character of
  platform/analysis-api/src/com/intellij/codeInsight/completion/util/
  ParenthesesInsertHandler.java. Only behaviour is used; reimplemented in
  Rust, no code is copied.
