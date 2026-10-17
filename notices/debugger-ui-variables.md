- crates/platform/debugger-ui (the frames and variables views of Threads &
  Variables and their icons), crates/platform/debugger/src/values.rs and
  session.rs (children and sizes of values), crates/python/py-debug/src/values.rs
  (Python values from debugpy), crates/platform/themes (the Debugger.Variables
  colours and NOT_TOP_FRAME_ATTRIBUTES),
  crates/platform/workbench/keymaps/macos.toml and default.toml (frame keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the icons of
  platform/icons/src/expui/debugger/ (frame, threadCurrent, dbPrimitive,
  dbArray, value, specialVar, watch, watchLastReturnValue, addToWatch,
  evaluationResult), light and dark, copied unchanged; the frames view of
  platform/xdebugger-impl/src/com/intellij/xdebugger/impl/frame/XFramesView.java
  and XDebuggerFramesList.java (the thread combo, frames of non-project files on
  the non-project background, the "Switch frames" hint); the variables tree of
  frame/XVariablesView.java, XVariablesViewBase.java, XDebugView.java (the
  100 ms clear), ui/tree/XDebuggerTree*.java, nodes/XValueNodeImpl.java,
  XValueContainerNode.java (100 children, then the ellipsis node) and
  XValuePresentationUtil.java (`name = {type} value`); the restored expansion
  and changed values of XDebuggerTreeState.java and XDebuggerTreeRestorer.java;
  the Python presentation of python/pydevSrc and python/src/com/jetbrains/python/
  debugger (PyDebugValue.java: the type with the container's size, the
  value's length limit; PyStackFrame.java: Special Variables and Return
  Values; PyDebugValueGroups.kt); the colours Debugger.Variables.* of the
  themes (intellijlaf.theme.json, darcula.theme.json, expUI_*.theme.json) and
  NOT_TOP_FRAME_ATTRIBUTES of the colour schemes; strings of
  XDebuggerBundle.properties and PyBundle.properties; the keys of
  PreviousOccurence and NextOccurence of platform/platform-resources/src/keymaps
  ($default.xml, Mac OS X 10.5+.xml). Changes: reimplemented in Rust with GPUI;
  the size of a container is evaluated in the frame with one expression per
  expanded node (debugpy reports no size); debugpy's own children (its
  "special variables", "(return)" values and "more" ranges) are mapped to the
  reference's groups and ellipsis node.
