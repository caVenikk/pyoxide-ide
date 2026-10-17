- crates/platform/search/, crates/platform/editor/src/find.rs
  The behaviour of Find and Replace in the editor follows the IntelliJ
  Platform Community (Apache-2.0), https://github.com/JetBrains/intellij-community
  @ f5a0bddb9cee9135c700de9a67534e11cdf61c07: the whole-word rule of
  platform/lang-impl/src/com/intellij/find/impl/FindManagerBase.java, the
  replacement template syntax of .../find/impl/RegExReplacementBuilder.java,
  the choice of the current match and the wrap-around of
  .../find/impl/livePreview/SearchResults.java and .../find/FindUtil.java,
  the multi-line fields of .../find/SearchTextArea.java (new-line key, three
  visible rows),
  and the UI strings of the find bar ("{0} results", "{0}/{1}", "{0}+",
  "Bad pattern", "Vague pattern", "\"{0}\" not found, press {1} to search
  from the top") from platform/ide-core/resources/messages/ApplicationBundle.properties
  and platform/analysis-impl/resources/messages/FindBundle.properties.
  Changes: reimplemented in Rust on the `regex` crate; word characters are
  Unicode letters, digits and `_`.
