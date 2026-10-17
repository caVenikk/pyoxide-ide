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
6. In the Add dialog choose Generate new, Virtualenv, and as the location
   a folder that holds an environment (the project's `.venv`), then a
   folder that exists without one (the project folder).
7. Make `uv` unavailable and choose Generate new, type uv.

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
- Step 6: a folder with an environment: "Already contains Python with
  version 3.13" with the link "Select existing interpreter" (it switches
  to Select existing with that interpreter); a folder without one: "The
  specified folder already exists". OK is disabled in both.
- Step 7: the "Path to uv:" field gives way to "No uv executable found"
  with the links "Install uv" and "Select path".

## Commands
`settings.open {"page": "python.interpreter"}`, `settings.edit {"field":
"interpreter", "value": "<path>" | null}`, `settings.edit {"field":
"list", "value": "interpreters" | "add"}`, `python.interpreter.add
{"kind": "existing-python", "python": ".venv"}`, `python.interpreter.add
{"kind": "existing-uv", "python": ".venv", "uv_path": "/path/to/uv",
"select": "now"}`, `python.interpreter.select {"name": ...}`,
`settings.apply`, `settings.state`, `python.interpreter.state`.

## Source
https://www.jetbrains.com/help/pycharm/configuring-python-interpreter.html,
https://www.jetbrains.com/help/pycharm/creating-virtual-environment.html;
IntelliJ Community (Apache-2.0) at `pycharm/2026.2.3`:
`PyActiveSdkConfigurable`, `PythonAddCustomInterpreter`,
`FileSystem.validateVenv` (step 6), `sdk.create.custom.venv.missing.text`
(step 7; the longer "Make sure uv is installed and added to PATH." text is
for remote targets). The page and dialog layouts are to verify against a
reference screenshot.
