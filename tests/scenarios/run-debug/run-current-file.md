# Run the current file

## Steps
1. Open a project with a virtual environment in `.venv` and open
   `main.py` (the only editor). Look at the run widget of the main toolbar.
2. Press Ctrl+R (macOS) or Shift+F10 (Windows, Linux), or click Run.
3. Run a script that prints `hi`, one that ends with `sys.exit(3)` and one
   that raises `ValueError`.
4. Edit the file without saving and run it again.
5. Press Ctrl+Shift+R (macOS) or Ctrl+Shift+F10.
6. Open a project without an interpreter and run a file.

## Expected
- Step 1: the widget shows "Current File" without an icon, then Run,
  Debug and More Actions; Run's tooltip is "Run 'main.py'".
- Step 2: all documents are saved; the Run tool window opens below the
  editor without taking the focus, with a tab "main" (the file name
  without `.py`); the first console line is the command line
  (`<project>/.venv/bin/python <project>/main.py `, with a trailing
  space) in the system output colour.
- Step 3: `hi` in the normal output colour, then an empty line, then
  "Process finished with exit code 0" in the system colour; the exit code
  3; the traceback in the error colour and "exit code 1".
- Step 4: the run uses the edited text (it was saved first).
- Step 5: the same as Run (the reference also creates a temporary run
  configuration "main"; that arrives with run configurations).
- Step 6: no process starts; the status bar reports "Error running 'main':
  Please select a valid Python interpreter" (the reference opens the run
  configuration editor with this error).

## Commands
`run.run`, `run.current-file`, `run.state` (`tabs[].console.lines`,
`widget`).

## Source
https://www.jetbrains.com/help/pycharm/running-applications.html,
https://www.jetbrains.com/help/pycharm/run-tool-window.html; observed in
PyCharm 2026.2.3; IntelliJ Platform Community (Apache-2.0)
`ExecutorAction`, `PythonRunConfigurationProducer`,
`ProcessTerminatedListener`.
