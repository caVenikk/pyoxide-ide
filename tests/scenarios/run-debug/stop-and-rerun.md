# Stop and Rerun

## Steps
1. Run a script that loops forever and press Cmd+F2 (macOS) or Ctrl+F2.
2. Run a script that ignores SIGINT
   (`signal.signal(signal.SIGINT, signal.SIG_IGN)`), press Stop, then
   Stop again.
3. Run a long script; in the Run tool window press Cmd+R (macOS) or
   Ctrl+F5; in the editor press Cmd+R.
4. After the process ended, press Rerun.

## Expected
- Step 1: the process gets SIGINT (Ctrl+C on Windows): Python prints a
  `KeyboardInterrupt` traceback and the console ends with "Process finished
  with exit code 130 (interrupted by signal 2:SIGINT)" (on Windows
  "-1073741510 (0xC000013A: interrupted by Ctrl+C)").
- Step 2: after the first Stop the tab's Stop button turns into "Kill
  process"; the second Stop kills the process tree: "exit code 137
  (interrupted by signal 9:SIGKILL)".
- Step 3: Rerun stops the process and starts the file again in the same
  tab without asking; Rerun is disabled while the process terminates. In
  the editor Cmd+R stays Replace.
- Step 4: the file runs again in the same tab.

## Commands
`run.stop`, `run.rerun`, `run.state` (`tabs[].status`, `exit_text`,
`can_rerun`).

## Source
Observed in PyCharm 2026.2.3; IntelliJ Platform Community (Apache-2.0)
`StopAction`, `KillableProcessHandler`, `FakeRerunAction`,
`ProcessTerminatedListener.stringifyExitCode`.
