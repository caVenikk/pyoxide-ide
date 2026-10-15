# The run widget of the main toolbar

## Steps
1. Close every editor; open `README.md`; open `main.py`. Hover Run each
   time.
2. Run a script that runs until it is stopped. Look at the widget.
3. Open another Python file and run it too; hover and click Stop.
4. Press Run again while `main.py` runs (with `main.py` in the editor).
5. Click the "Current File" selector and More Actions.

## Expected
- Step 1: Run is disabled with the tooltip "Cannot run the current file:
  the editor is not in focus" without an editor and "The file in the
  editor is not runnable" for `README.md`; enabled with "Run 'main.py'"
  for `main.py`.
- Step 2: Run shows the Rerun icon on a filled green button; Stop appears
  on a filled red button (its place is empty while nothing runs) with the
  tooltip "Stop 'main'"; Debug stays.
- Step 3: Stop carries a count badge ("2") and the tooltip "Stop..."; a
  click opens "Stop ‘main’", "Stop ‘other’", a separator and "Stop All 2".
- Step 4: the dialog "Process 'main' Is Running" with a question icon,
  "'main' is not allowed to run in parallel. Would you like to stop the
  running one?", a "Do not show this dialog in the future" check box and
  [Cancel] [Stop and Rerun] (the default); Stop and Rerun stops the
  process and runs the file again in the same tab.
- Step 5: in a project without run configurations the dropdown lists
  "Current File", a separator and "Edit Configurations…" (disabled until
  the Run/Debug Configurations dialog exists; with configurations see
  run-configurations.md); More Actions offers "Run with Parameters…"
  (disabled likewise).

## Commands
`run.run`, `run.stop {"tab": n}`, `run.stop-all`, `run.configurations`,
`run.more-actions`, `run.select-current-file`, `dialog.confirm`,
`dialog.cancel`, `run.state` (`widget`, `menu`), `workbench.state`
(`dialog`).

## Source
Observed in PyCharm 2026.2.3; IntelliJ Platform Community (Apache-2.0)
`RedesignedRunWidget`, `RunToolbarPopup`, `StopAction`,
`ExecutionManagerImpl`.
