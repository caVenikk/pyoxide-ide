- crates/platform/debugger-ui/src/watches.rs and variables.rs (the evaluate
  field and its history list, the watches, Pause Watch, Remove All Watches, Set
  Value, Copy Value, Copy Name, Jump To Source, Jump To Type Source, View as and
  the context menu of Threads & Variables), crates/platform/debugger-ui/assets/icons/values/evaluateExpression*.svg,
  crates/platform/debugger/src/debugger.rs, breakpoints.rs and storage.rs (the
  histories, paused watches), crates/python/py-debug/src/declaration.rs and
  expressions.rs (where a Python value is declared, the names an expression does
  not declare, failed evaluations, the integer renderers),
  crates/platform/workbench/keymaps/macos/debug.toml and default/debug.toml (their
  keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the evaluate field and the
  watches of platform/xdebugger-impl/ui/src/com/intellij/xdebugger/impl/frame/
  XWatchesViewImpl.java, WatchInplaceEditor.java, actions/XNewWatchAction.java,
  XRemoveWatchAction.java, XRemoveAllWatchesAction.java, XEditWatchAction.java,
  XPauseWatchAction.kt; shared/src/com/intellij/xdebugger/impl/XWatch.kt (paused
  watches); ui/XDebuggerEmbeddedComboBox.java, XDebuggerExpressionComboBox.java
  (the error outline of an expression with errors, ten rows of history) and
  XDebuggerHistoryManager.java (a history per editor, 50 expressions,
  `debugger.max.recent.expressions`); ui/tree/nodes/WatchesRootNode.java (the
  result node first, then the watches) and WatchNodeImpl.java (the paused
  presentation); ui/tree/SetValueInplaceEditor.java, XDebuggerTreeInplaceEditor.java
  and XDebuggerTreeValueNodeInplaceEditor.java; ui/tree/actions/XSetValueAction.java,
  XCopyValueAction.java, XCopyNameAction.java, XAddToWatchesTreeAction.java,
  XJumpToSourceAction.kt, XJumpToTypeSourceAction.java; the action groups of
  ui/resources/intellij.platform.debugger.impl.ui.xml (XDebugger.Watches.Tree.Popup,
  XDebugger.ValueGroup) and of python/pluginResources/intellij.python.community.impl.xml
  (PyDebugger.CustomizeDataView, PyDebugger.ViewAsGroup); the integer renderers
  of python/pydevSrc/src/com/jetbrains/python/debugger/render/ (Binary, Decimal,
  Hex) and python/src/com/jetbrains/python/debugger/actions/ViewAsGroup.java; the
  action texts of XDebuggerUiBundle.properties and XDebuggerBundle.properties
  (`xdebugger.watch.is.paused.message`, `xdebugger.evaluate.paused.watch`); the
  icon platform/icons/src/expui/run/evaluateExpression.svg and its dark variant
  (`AllIcons.Debugger.Db_evaluateNode` of the result node), copied unchanged; the
  watches' key of src/com/intellij/xdebugger/impl/XDebugSessionImpl.kt (the run
  configuration type); Jump To Source of a Python value after
  python/pydevSrc/src/com/jetbrains/python/debugger/PyDebugValue.java
  (computeSourcePosition); the keys of XDebugger.NewWatch, XDebugger.SetValue,
  XDebugger.JumpToTypeSource, EvaluateExpression, EditSource, $Delete and $Copy of
  platform/platform-resources/src/keymaps ($default.xml, Mac OS X 10.5+.xml).
  Changes: reimplemented in Rust with GPUI; the histories and the watches are kept
  in `.oxide/workspace.toml`; Evaluate Expression of a value puts it into the
  evaluate field (there is no Evaluate dialog); the declaration of a Python name
  and the names an expression does not declare are found by a scan of the frame's
  scope and the module instead of the reference's resolver; Jump To Type Source
  asks the running program (`inspect`) where a type is; a failed evaluation is
  shown as pydevd shows it, from debugpy's message; the result node's icon
  replaces debugger/evaluationResult of debugger-ui-variables.md.
