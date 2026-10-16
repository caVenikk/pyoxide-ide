# Python > Interpreter

## Steps
1. Open Settings > Python > Interpreter in a uv project; open the
   "Python Interpreter:" combo.
2. Open "Add Interpreter" ▸ Add Local Interpreter…; choose Generate new,
   type uv, keep the location; press OK.
3. Choose "Show All…" in the combo; rename an interpreter; try a name
   that already exists.
4. Choose another interpreter and press Apply.
5. Open another project and its interpreter combo.
6. In the Add dialog choose Generate new, Virtualenv, a location that
   already exists.
7. Make `uv` unavailable and choose type uv.

## Expected
- Step 1: the page carries the project-level mark; the combo lists "No
  interpreter", the project's environments, system interpreters, a
  separator and "Show All…".
- Step 2: the environment is created off the UI thread (the link is
  disabled with "An interpreter is being configured." meanwhile), added
  to the application list and selected.
- Step 3: the "Python Interpreters" dialog lists the configured
  interpreters; renaming works; a duplicate name is refused.
- Step 4: the project uses it; the status bar widget follows.
- Step 5: the new interpreter is offered there too, unless it is
  associated with the first project only.
- Step 6: "The specified folder already exists".
- Step 7: "No uv executable found. Make sure uv is installed and added to
  PATH."

## Commands
`settings.open {"page": "python.interpreter"}`, `python.interpreter.add
{"kind": ..., ...}`, `python.interpreter.select {"name": ...}`,
`settings.apply`, `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/configuring-python-interpreter.html,
https://www.jetbrains.com/help/pycharm/creating-virtual-environment.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`PyActiveSdkConfigurable`, `PythonAddCustomInterpreter`. The page and
dialog layouts are to verify against a reference screenshot.
