- crates/platform/debugger-ui/src/watches.rs and variables.rs (the evaluate
  field, the watches, Set Value, Copy Value, Copy Name, Jump To Source and the
  context menu of Threads & Variables), crates/platform/debugger/src/debugger.rs
  and storage.rs (the expression history), crates/python/py-debug/src/declaration.rs
  (where a Python value is declared), crates/platform/workbench/keymaps/macos/debug.toml
  and default/debug.toml (their keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the evaluate field and the
  watches of platform/xdebugger-impl/ui/src/com/intellij/xdebugger/impl/frame/
  XWatchesViewImpl.java, WatchInplaceEditor.java, actions/XNewWatchAction.java,
  XRemoveWatchAction.java, XEditWatchAction.java; ui/XDebuggerEmbeddedComboBox.java
  and XDebuggerHistoryManager.java (the history, 50 expressions,
  `debugger.max.recent.expressions`); ui/tree/nodes/WatchesRootNode.java (the
  result node first, then the watches) and WatchNodeImpl.java;
  ui/tree/SetValueInplaceEditor.java, XDebuggerTreeInplaceEditor.java and
  XDebuggerTreeValueNodeInplaceEditor.java; ui/tree/actions/XSetValueAction.java,
  XCopyValueAction.java, XCopyNameAction.java, XAddToWatchesTreeAction.java,
  XJumpToSourceAction.kt; the action groups of
  ui/resources/intellij.platform.debugger.impl.ui.xml (XDebugger.Watches.Tree.Popup,
  XDebugger.ValueGroup) and the action texts of XDebuggerUiBundle.properties;
  the watches' key of src/com/intellij/xdebugger/impl/XDebugSessionImpl.kt (the
  run configuration type); Jump To Source of a Python value after
  python/pydevSrc/src/com/jetbrains/python/debugger/PyDebugValue.java
  (computeSourcePosition); the keys of XDebugger.NewWatch, XDebugger.SetValue,
  EditSource, $Delete and $Copy of platform/platform-resources/src/keymaps
  ($default.xml, Mac OS X 10.5+.xml). Changes: reimplemented in Rust with GPUI;
  the history and the watches are kept in `.oxide/workspace.toml`; Evaluate
  Expression of a value puts it into the evaluate field (there is no Evaluate
  dialog); the declaration of a Python name is found by a scan of the frame's
  scope instead of the reference's resolver.
