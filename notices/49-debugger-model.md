- crates/platform/debugger/src/idea.rs, commands.rs, docs/adr/0015-debugger-over-dap.md
  The debugger model follows the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  404a109fe630a2b9013f10b71081f87c9dbb32e2 (tag pycharm/2026.2.3): the
  `XDebuggerManager` component of `.idea/workspace.xml` that the one-time
  import reads (the persisted breakpoint and watches state of
  platform/xdebugger-impl: xdebugger/impl/breakpoints/BreakpointState.java,
  LineBreakpointState.java, XBreakpointManagerImpl.java and
  xdebugger/impl/XDebuggerWatchesManagerImpl.java), and the titles of the
  `debug.*` commands, which use the action texts of XDebuggerBundle and
  ActionsBundle. Only the file format and strings are used; reimplemented
  in Rust, no code is copied, and `.idea` is only read. The Debug Adapter
  Protocol client is written from the public specification.
