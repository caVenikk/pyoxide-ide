- crates/python/py-debug/src/lines.rs, breakpoints.rs, launch.rs
  Follow IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the stoppable-line rule of
  python/src/com/jetbrains/python/debugger/PyLineBreakpointType.java
  (canPutAt, its unstoppable elements and token types, the default suspend
  policy) with PyDebugSupportUtils.java (isContinuationLine) and
  platform/xdebugger-impl/src/com/intellij/xdebugger/impl/
  XDebuggerUtilImpl.java (iterateLine), using the tokens of
  python/python-parser/src/com/jetbrains/python/lexer/Python.flex and the
  backslash handling of PythonIndentingProcessor.java; the type ids, the
  default "Any exception" breakpoint and the attribute names of
  PyExceptionBreakpointType.java and PyExceptionBreakpointProperties.java;
  the defaults of settings/PyDebuggerSettings.java and
  PyDebuggerOptionsProvider.java; the string "Any exception" of
  python/pluginResources/messages/PyBundle.properties. Reimplemented in
  Rust, no code is copied.
