- crates/platform/name-match, crates/platform/file-index,
  crates/platform/workbench/src/search_everywhere/actions.rs,
  crates/platform/workbench/src/keymap/gesture.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  0b33b8b8e90353a31a9db5b8fcecc6626d94967e. Files:
  platform/util/text-matching/src/com/intellij/psi/codeStyle/
  MinusculeMatcher.kt, MinusculeMatcherImpl.kt, NameUtil.java,
  FixingLayoutMatcher.kt, MatcherWithFallback.kt,
  PreferStartMatchMatcherWrapper.kt, WordPrefixMatcher.kt;
  platform/util/text-matching/src/com/intellij/util/text/matching/
  KeyboardLayoutConverter.kt (the Russian layout table);
  platform/util/base/src/com/intellij/util/text/NameUtilCore.kt (word
  starts); platform/lang-impl/src/com/intellij/ide/actions/
  GotoFileItemProvider.java and DirectoryPathMatcher.java;
  platform/lang-impl/src/com/intellij/ide/util/gotoByName/
  DefaultChooseByNameItemProvider.java, GotoFileModel.java,
  ChooseByNamePopup.java, ActionSearchUtil.kt and GotoActionModel.java;
  platform/lang-impl/src/com/intellij/psi/util/proximity/ (the order of
  the proximity weighers); platform/lang-impl/src/com/intellij/ide/actions/
  searcheverywhere/SearchEverywhereNavigationHandler.kt (the line and
  column pattern); platform/platform-impl/src/com/intellij/openapi/keymap/
  impl/ModifierKeyDoubleClickHandler.kt (the double tap's timing).
  Test cases from platform/util/text-matching/test/com/intellij/platform/
  util/text/matching/NameUtilMatchingTest.kt and java/java-tests/testSrc/
  com/intellij/java/navigation/ChooseByNameTest.java and
  ChooseByNameHddTest.java.
  Adapted: the matching algorithm and degree, Go to File's rounds over
  name suffixes and folder patterns, its order and weights, absolute
  paths, the action weights, the double tap's rules and times.
  Changes: reimplemented in Rust over char slices; no Pinyin, typo-tolerant
  or fuzzy (Levenshtein) matching, no kana word breaks; the proximity
  order keeps open files, the current file's folder and project content
  (there are no modules, source roots or usage statistics).
