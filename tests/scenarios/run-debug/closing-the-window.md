# Closing the window ends what it started

## Steps
1. Open the Breakpoints window (Cmd+Shift+F8 on macOS, Ctrl+Shift+F8),
   then close the main window with nothing running.
2. Run a long script, debug another one that stops at a breakpoint, open
   a terminal tab and start `sleep 600` in it; close the main window (or
   File | Close Project) and answer Cancel.
3. Close it again and answer Terminate twice.
4. Repeat step 2, close the window, check "Don't ask again" and answer
   Disconnect.

## Expected
- Step 1: the window closes at once, the Breakpoints window with it.
- Step 2: a dialog "Process 'x' Is Running" / "Do you want to terminate
  the process 'x'?" with Terminate, Disconnect and Cancel and "Don't ask
  again", once per running process (the run first, then the debug
  session). Cancel keeps the window and everything in it running.
- Step 3: the window closes once the processes ended: no run process, no
  debugged program, no debug adapter (`pgrep -fl debugpy` lists nothing)
  and nothing started in the terminal remains; the project's background
  tasks are cancelled.
- Step 4: the run's process and the debugged program go on running
  without the IDE (the debugger detaches); the debug session is not asked
  about. The choice is kept for closing Run tabs too.
- Quit (Cmd+Q) asks the same way in every window; Cancel stops quitting.

## Commands
`project.close`, `dialog.choose`, `dialog.toggle-option`,
`workbench.state` (`dialog`), `run.state`, `debug.state`.

## Source
IntelliJ Platform Community (Apache-2.0) at `pycharm/2026.2.3`:
`BaseContentCloseListener.canClose`, `TerminateRemoteProcessDialog`,
`ExecutionBundle` (`process.is.running.dialog.title`,
`terminate.process.confirmation.text`).
