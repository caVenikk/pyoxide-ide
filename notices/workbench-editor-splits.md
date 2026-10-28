- crates/platform/workbench/src/split_tree.rs and
  crates/platform/workbench/src/workbench/splits.rs (the split windows of
  the editor area, Split Right / Down, Split and Move, Unsplit, Change
  Splitter Orientation, Go to Next / Previous Splitter)
  Follow the IntelliJ Platform Community (Apache-2.0, license text in
  LICENSE-APACHE), https://github.com/JetBrains/intellij-community @ tag
  pycharm/2026.2.3: EditorWindow.kt (split, removeFromSplitter, closeFile),
  EditorsSplitters.kt (getOrderedWindows, createSplitter with the proportion
  0.5 limited to 0.1..0.9) of platform/platform-impl/src/com/intellij/
  openapi/fileEditor/impl/, SplitAction.java and SplitterAction.java of
  platform/platform-impl/src/com/intellij/ide/actions/, the group
  EditorTabsGroup of platform/platform-resources/src/idea/
  PlatformActions.xml, the texts of platform/platform-resources-en/src/
  messages/ActionsBundle.properties and the bindings of
  platform/platform-resources/src/keymaps/$default.xml (NextSplitter,
  PrevSplitter). Only behaviour and texts are used; reimplemented in Rust,
  no code is copied.
