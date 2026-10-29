- crates/platform/debugger-ui/src/evaluate.rs (the Evaluate dialog),
  crates/platform/debugger/src/debugger.rs and storage.rs (the code fragment
  history and the mode kept with the project),
  crates/platform/workbench/keymaps/macos/debug.toml and default/debug.toml (the
  dialog's keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the dialog
  of platform/xdebugger-impl/ui/src/com/intellij/xdebugger/impl/evaluate/
  XDebuggerEvaluationDialog.java (non-modal window, the two modes and Alt+M,
  Add to Watches with the key of getAddWatchKeystroke, Alt+R to the result, the
  result tree expanded and selected on its first node, evaluation again after a
  run of the code, close with the session, the mode remembered),
  ExpressionInputComponent.java (the field with the expand button, the hint
  under it), CodeFragmentInputComponent.java (the multi-line editor with the
  collapse button over the result) and the evaluation action handler
  (XDebuggerEvaluateActionHandler: the text of the selection, the word at the
  caret or the selected value's expression); the texts of
  platform/xdebugger-api/resources/messages/XDebuggerBundle.properties (`Evaluate`,
  `Expression:`, `Code fragment:`, `Result:`, `Evaluating…`, `Use {0} to
  add to Watches`, `Cannot evaluate, current stack frame doesn't support
  evaluation`).
  Changes: reimplemented in Rust with GPUI over the debugger model; the
  evaluation runs as a `repl` evaluation of the debug adapter in both modes;
  the result is a tree of the popup kind (`VariablesView::for_popup`) without
  the reference's context menu; the history of the expression mode is the
  evaluate field's, the code fragment mode has its own (`evaluateCodeFragment`);
  the language chooser of the field is not drawn; the collapse button's icon is
  drawn anew in the style of the expand button.
