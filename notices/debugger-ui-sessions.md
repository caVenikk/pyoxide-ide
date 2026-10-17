- crates/platform/debugger-ui (debug sessions, the Debug tool window, the
  execution point and their icons), crates/platform/console-ui,
  crates/platform/debugger/src/session.rs (Run to Cursor, Step Into My Code),
  crates/python/py-debug/src/launch.rs and provider.rs (Step Into My Code with
  debugpy), crates/platform/workbench/src/run (the run widget's Debug, Resume
  and Pause, the dropdown's and the Run/Debug Configurations dialog's Debug),
  crates/platform/workbench/src/workbench/debug.rs and run_actions.rs ("Process
  'x' Is Running", Stop shared with Run),
  crates/platform/workbench/keymaps/macos.toml and default.toml (debugger keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the icons of
  platform/icons/src/expui/run/ (restartDebug, restartDebug_stroke, resume,
  resume_stroke, pause, stepOver, stepInto, stepOut, viewBreakpoints,
  muteBreakpoints, runToCursor, forceRunToCursor, showCurrentFrame),
  expui/debugger/stepIntoMyCode and expui/general/layout, light and dark,
  copied unchanged; the session tab and its toolbar of platform/xdebugger-impl/
  src/com/intellij/xdebugger/impl/ui/XDebugSessionTab.java and
  XDebugSessionTabNewUI.kt with the toolbar groups of
  intellij.platform.debugger.impl.content.xml; the execution line of
  ExecutionPointHighlighter.java (EXECUTIONPOINT_ATTRIBUTES); the Debug
  executor of platform/xdebugger-api/src/com/intellij/execution/executors/
  DefaultDebugExecutor.java; the run widget's states of platform/execution-impl/
  src/com/intellij/execution/ui/RedesignedRunWidget.kt
  (filterOutRunIfDebugResumeIsPresent, RunWidgetButtonLook), actions/
  ExecutorAction.java and platform/xdebugger-impl/src/com/intellij/xdebugger/
  impl/runToolbar/RunToolbarXDebuggerAction.kt with actions/handlers/
  RunToolbarDebugActionHandler.kt; the dropdown's inline actions of
  RunToolbarPopup.kt (createRunConfigurationWithInlines,
  createCurrentFileWithInlineActions, createStopPopup); "Process 'x' Is
  Running" and Stop and Rerun of impl/ExecutionManagerImpl.kt
  (restartRunProfile, userApprovesStopForSameTypeConfigurations); Step Into My
  Code of python/pluginResources/intellij.python.community.impl.xml
  (PyStepIntoMyCodeAction); strings of ExecutionBundle.properties,
  XDebuggerBundle.properties, IdeBundle.properties (Resume and Pause of the
  run widget) and PyBundle.properties (the connection failure title); the keys
  of Debug, DebugClass, ActivateDebugToolWindow, Resume, StepOver, StepInto,
  StepIntoMyCode, StepOut, RunToCursor, ForceRunToCursor, ShowExecutionPoint,
  Stop and Rerun of platform/platform-resources/src/keymaps ($default.xml, Mac
  OS X.xml, Mac OS X 10.5+.xml). Changes: reimplemented in Rust with GPUI; Step
  Into My Code turns pydevd's library filter on for the one step through an
  evaluation and back to its value at the next stop, as debugpy 1.8.21 takes
  no "just my code" switch over the protocol; Run to Cursor is a breakpoint of the session only; the Threads &
  Variables tab lists the frames until the frames and variables views come.
