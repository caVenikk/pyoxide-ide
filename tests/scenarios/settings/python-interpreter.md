# Python > Interpreter

## Steps
1. Open Settings > Python > Interpreter in a uv project; open the
   "Python Interpreter:" combo.
2. Open "Add Interpreter" ▸ Add Local Interpreter…; choose Generate new,
   type uv, keep the location; press OK.
3. Choose "Show All…" in the combo; rename an interpreter; try a name
   that already exists, then an empty one; toggle the filter; remove one
   and press Cancel; select one and press OK.
4. Choose another interpreter and press Apply.
5. Open another project and its interpreter combo.
6. In the Add dialog choose Generate new, Virtualenv, and as the location
   a folder that holds an environment (the project's `.venv`), then a
   folder that exists without one (the project folder).
7. Make `uv` unavailable and choose Generate new, type uv.
8. With uv available, in the Add dialog open the Base Python list
   (Virtualenv) and the Python version list (uv); choose a Python marked
   "download and install" and press OK.

## Expected
- Step 1: the page carries the project-level mark; the combo lists "No
  interpreter", the project's environments, system interpreters, a
  separator and "Show All…".
- Step 2: the environment is created off the UI thread (the link is
  disabled with "An interpreter is being configured." meanwhile), added
  to the application list and selected.
- Step 3: the "Python Interpreters" dialog (800 × 648) lists the
  configured interpreters and the project's environments by name under a
  toolbar (Add New Interpreter, Remove Interpreter, Rename, "Hide virtual
  environments associated with other projects", Show Interpreter Paths);
  the details at the right show "Interpreter path:" of the selected one.
  Rename opens "Rename Python Interpreter" ("New name:"); a duplicate
  name shows "Python interpreter with the same name already exists", an
  empty one "Provide a name for a Python interpreter", and OK stays
  disabled. The filter hides environments associated with other
  projects. Remove waits for Apply/OK; Cancel keeps the interpreter. OK
  selects the chosen interpreter in the page until its Apply.
- Step 4: the project uses it; the status bar widget follows.
- Step 5: the new interpreter is offered there too, unless it is
  associated with the first project only.
- Step 6: a folder with an environment: "Already contains Python with
  version 3.13" with the link "Select existing interpreter" (it switches
  to Select existing with that interpreter); a folder without one: "The
  specified folder already exists". OK is disabled in both.
- Step 7: a warning row "No uv executable found" with the links "Install
  uv" and "Select path" stands above the empty "Path to uv:" field; OK is
  disabled. With uv found the field shows its path and a green check.
- Step 8: after the installations found, the Base Python list shows a
  row per version none of them has, with the download icon, "Python
  3.13.5" and the grey " download and install" (uv's newest CPython of
  each version from 3.8 on, no free-threaded or PyPy builds); the uv
  versions not installed carry the same grey text. OK runs `uv python
  install <version>` first (progress "Installing Python <version>"), then
  makes the environment with the installed Python (`uv python find`) or,
  for uv, with `uv venv --python <version>`.
- Steps 2 and 6, uv: the project's `.venv` as the location gives the
  alert "Environment at ".venv" already exists" with "Override existing
  environment" (it turns into the warning "Existing environment at
  ".venv" will be overridden" and enables OK) and "Select existing
  interpreter".

## Commands
`settings.open {"page": "python.interpreter"}`, `settings.edit {"field":
"interpreter", "value": "<path>" | null}`, `settings.edit {"field":
"list", "value": "interpreters" | "add"}`, `python.interpreter.add
{"kind": "existing-python", "python": ".venv"}`, `python.interpreter.add
{"kind": "existing-uv", "python": ".venv", "uv_path": "/path/to/uv",
"select": "now"}`, `python.interpreter.select {"name": ...}`,
`settings.apply`, `settings.state`, `python.interpreter.state` (its
`add-dialog` is the open dialog). The dialog: `python.interpreter.add-local`,
`python.interpreter.add-dialog.set {"field": "type", "value": "uv"}`
(fields `environment`, `type`, `base-python`, `python`, `uv-environment`,
`location`, `uv-path`, `python-version`, `inherit-site-packages`,
`available-to-all`), `python.interpreter.add-dialog.open-list {"list":
"type"}`, `python.interpreter.add-dialog.override`,
`python.interpreter.add-dialog.select-existing`,
`python.interpreter.add-dialog.ok`, `python.interpreter.add-dialog.cancel`
(a Python to download is the `base-python` value `download:3.13.5`);
without it `python.interpreter.add {"kind": "uv", "location": ".venv",
"python_version": "3.13"}`, `{"kind": "virtualenv", "base_python":
"/usr/bin/python3", "location": "/path/to/env"}` or `{"kind":
"virtualenv", "install_python": "3.13.5", "location": "/path/to/env"}`.
The Python Interpreters dialog: `python.interpreters.show-all`,
`python.interpreters.dialog.select {"name": ...}`,
`python.interpreters.dialog.rename`, `python.interpreters.rename-dialog.set
{"text": ...}`, `python.interpreters.rename-dialog.ok`,
`python.interpreters.dialog.toggle-filter`,
`python.interpreters.dialog.remove`, `python.interpreters.dialog.show-paths`,
`python.interpreters.dialog.apply`, `python.interpreters.dialog.ok`,
`python.interpreters.dialog.cancel`; without it
`python.interpreters.rename {"name": ..., "new_name": ...}`,
`python.interpreters.remove {"name": ...}`.

## Source
https://www.jetbrains.com/help/pycharm/configuring-python-interpreter.html,
https://www.jetbrains.com/help/pycharm/creating-virtual-environment.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`PyActiveSdkConfigurable`, `PythonAddCustomInterpreter`,
`PythonInterpreterMasterDetails` and `MasterDetailsComponent` (step 3),
`getInstallableInterpreters` and `installBaseSdk` (step 8; the reference
offers python.org installers, Pyoxide uv's downloads),
`FileSystem.validateVenv` (step 6), `sdk.create.custom.venv.missing.text`
and `ValidatedPathField.validatablePathField` (step 7; the longer "Make
sure uv is installed and added to PATH." text and the hidden field are
for remote targets). Layouts checked against reference screenshots of the
page, the Add dialog and its lists, and the Python Interpreters dialog.
