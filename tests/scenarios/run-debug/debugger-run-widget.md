# Debug from the run widget and configurations

## Steps
1. Select a Python run configuration with script parameters, environment
   variables and a working directory; hover the Debug button of the run
   widget.
2. Press Debug; with a breakpoint hit, look at the widget; resume and
   look again.
3. Open the widget's dropdown and hover a configuration row.
4. Open Edit Configurations… and debug from the dialog.
5. With a single-instance configuration being debugged, press Debug
   again.

## Expected
- Step 1: the tooltip "Debug '<name>'".
- Step 2: the parameters, variables and working directory are applied;
  while suspended the Run slot shows "Resume '<name>'", while running
  "Pause '<name>'"; Debug becomes Rerun; Stop is shown.
- Step 3: the row has an inline "Debug '<name>'" button.
- Step 4: the dialog's Debug starts the selected configuration.
- Step 5: the dialog "Process '<name>' Is Running" from the Run flow.

## Commands
`debug.debug`, `debug.choose-configuration`, `run.configurations`,
`run.state` (`widget`, `menu`), `debug.state`.

## Source
https://www.jetbrains.com/help/pycharm/debugging-code.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `RedesignedRunWidget`,
`DefaultDebugExecutor`. The widget states are to verify against a
reference screenshot.
