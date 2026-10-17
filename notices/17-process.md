- crates/platform/process
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  82a18fd6f8095cced5d0cef964c041f63ece8ee5 (branch 262): the stop
  semantics of platform/platform-util-io/src/com/intellij/execution/process/
  KillableProcessHandler.java, OSProcessHandler.java, OSProcessUtil.java and
  platform/util/src/com/intellij/execution/process/UnixProcessManager.java
  (SIGINT, then SIGKILL, to the process tree, children first), of
  platform/execution-impl/src/com/intellij/execution/actions/StopAction.java,
  and of platform/platform-util-io-impl/src/com/intellij/execution/process/
  LocalProcessServiceImpl.kt (a 0xFF byte on standard input after Ctrl+C on
  Windows); the end of output of platform/util/src/com/intellij/execution/
  process/BaseOSProcessHandler.java; the exit code text of
  platform/ide-core/src/com/intellij/execution/process/
  ProcessTerminatedListener.java with the signal table of
  platform/util/base/src/com/intellij/execution/process/UnixSignal.kt and
  the message of platform/ide-core/resources/messages/IdeCoreBundle.properties;
  the command line presentation of platform/execution/src/com/intellij/
  execution/target/TargetedCommandLine.kt, platform/util/src/com/intellij/
  execution/CommandLineUtil.java and .../util/execution/ParametersListUtil.java.
  The Windows process tree kill and Ctrl+C follow WinP (MIT, (c) 2008-
  Kohsuke Kawaguchi, Sun Microsystems, Inc., and contributors),
  https://github.com/kohsuke/winp, files native/winp.cpp
  (KillProcessTreeWinHelper) and native/sendctrlc/main.cpp.
  Changes: reimplemented in Rust; the process starts in its own process
  group, which a kill also signals; the Windows Ctrl+C helper is the
  application itself, started with a hidden argument.
