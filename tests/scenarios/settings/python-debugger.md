# Python > Debugger

## Steps
1. Open a Python project; open Settings > Python > Debugger.
2. Turn "Attach to subprocess automatically while debugging" off and
   "Gevent compatible" on; open the list next to "PyQt compatible" and
   choose PyQt5; click the up arrow of "Debugger evaluation response
   timeout (ms):" once; press Apply.
3. Start a debug session of a run configuration.
4. Turn "PyQt compatible" off.
5. In the Settings search type `to subprocess automatically`.
6. Open the project of a PyCharm user whose `.idea/workspace.xml` has
   `PyDebuggerOptionsProvider` options, then Settings > Python > Debugger.

## Expected
- Step 1: under Python, after Interpreter and Type Engine and before
  Tools, the page "Debugger" with the project mark. Check boxes "Attach to
  subprocess automatically while debugging" (on), "Collect run-time types
  information for code insight" (off, with a warning icon and the link
  "Clear caches"), "Gevent compatible" (off), "Drop into debugger on
  failed tests" (off), "PyQt compatible" (on) with a combo box "Auto"; the
  field "For **Attach To Process** show processes with names containing:"
  (`python`); "Debugger evaluation response timeout (ms):" with a spinner
  (60000); "Mode:" with the segmented button pydevd | debugpy (debugpy
  chosen) and the hint "Python debugger backend built on the Debug Adapter
  Protocol, with a more stable connection model. Lower overhead on Python
  3.12+.". "Collect run-time types information", its warning and link,
  "Drop into debugger on failed tests", the Attach To Process field and
  pydevd are drawn dim and take no clicks (not supported yet).
- Step 2: the spinner shows 60500. PyQt6 and PySide6 are listed dim in
  the combo's list (debugpy has no Qt 6 support). Apply writes
  `[python.debugger]` to `.oxide/workspace.toml`: `attach-to-subprocess =
  false`, `gevent = true`, `pyqt-backend = "pyqt5"`, `evaluation-timeout =
  60500`, and `imported-from-idea = true`; options at their defaults are
  not written. A text that is no number from 0 to 2147483647 in the
  spinner turns its frame red and Apply shows "Please enter a number from
  0 to 2147483647".
- Step 3: the `launch` arguments have `subProcess: false`, `gevent: true`,
  `qt: "pyqt5"`; an evaluation (a watch, Evaluate Expression) the adapter
  does not answer in 60.5 s fails with "timeout waiting for a response
  from the debugger (60500 ms)" and the adapter is told to cancel it; 0
  waits for ever. The options are read again at each start.
- Step 4: the combo box is dim and does not open; after Apply the launch
  has `qt: "none"`.
- Step 5: the tree keeps Python > Debugger; the first check box is lit with
  the search spotlight.
- Step 6: the page shows the options of the reference's file (a project
  with `myAttachToSubprocess` off has the check box off). They are
  written to `[python.debugger]` at once, so later changes of the
  reference's file do not reach Pyoxide, and `.idea` is never written.

## Commands
`settings.open {"page": "python.debugger"}`, `settings.edit`
(`attach-to-subprocess`, `gevent`, `pyqt`, `pyqt-backend`,
`evaluation-timeout`, `list`), `settings.state`, `settings.apply`,
`settings.revert`, `settings.search`.

## Source
https://www.jetbrains.com/help/pycharm/settings-debugger.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `PyDebuggerConfigurable`,
`PyDebuggerOptionsProvider` (a project service stored in
`workspace.xml`: `myAttachToSubprocess`, `mySupportGeventDebugging`,
`mySupportQtDebugging`, `myPyQtBackend`, `myEvaluationResponseTimeout`),
`PyDebugProcess.evaluate`, `RemoteDebugger.waitForResponse`,
`PyCharmProjectConfigurableStartupActivity` (the page's place and weight
80 in the Python group), `PyBundle.properties`. Compared with a reference
screenshot of Settings > Python > Debugger (see the pull request for the
differences left).
