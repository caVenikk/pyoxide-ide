# Run configurations and their import from the reference IDE

## Steps
1. Open a project the reference IDE has used, with per-user
   configurations in `.idea/workspace.xml` (a Python script with an env
   file, a module run, one in a folder, a temporary one, a pytest one),
   a shared one in `.idea/runConfigurations/` and one in `.run/*.run.xml`;
   one of the per-user ones has the name of the shared one.
2. Open the run widget's dropdown; hover a configuration; type part of a
   name.
3. Run the selected configuration with Ctrl+R (macOS) or Shift+F10.
4. In a Python file, press Ctrl+Shift+R (macOS) or Ctrl+Shift+F10 in six
   files one after another; then in the first of them again.
5. In a row's submenu choose Save Configuration, Pin, then Delete.
6. Open Edit Configurations…: select a configuration, change its fields
   and Modify options (Option+M / Alt+M), Apply; Add (Cmd+N / Alt+Insert),
   Copy (Cmd+D / Ctrl+D), Create New Folder, Remove (Delete), Sort; turn on
   "Store as project file" for one with personal variable values; OK.
   Open it again, change something, Cancel.
7. Select the pytest configuration; give a Python configuration an
   interpreter name that does not exist and run it.
8. Reopen the project; delete `.oxide/workspace.toml` and reopen it.

## Expected
- Step 1: one notification "Imported N run configurations (M kept
  read-only)", naming the renamed per-user configuration
  (`name (1)`). The widget shows the configuration the reference had
  selected, with its type's icon (a temporary one at 45 % opacity).
  `.idea` and the `*.run.xml` files are unchanged; `.oxide/run/` holds a
  file per shared configuration, `.oxide/project.toml` marks the shared
  import, `.oxide/workspace.toml` (ignored by Git) holds the personal
  configurations and every personal variable value.
- Step 2: with fewer than 5 configurations or no start history, every
  configuration by type, in the reference's order (type name, folders
  first, permanent before temporary, name or the custom order), a
  separator after each type, folders as submenus with the folder icon;
  then "Current File", a separator, "Edit Configurations…". With 5 or more
  and a history: "Pinned" (when something is pinned), "Recent
  Configurations" and "All Configurations  N" with a chevron, which shows
  the rest. A configuration's row selects it on a click; while it is
  selected it shows Run (Rerun while it runs), Stop while it runs, and
  Debug (disabled), and its submenu: Run, Debug, "Configuration", Edit
  Configurations…, Save Configuration (temporary ones), Delete, Pin or
  Unpin. An unsupported one says "not supported yet" and has no buttons.
  Typing filters and also finds the configurations inside folders and
  behind the toggle, with their folder after the name. The dropdown is
  310 to 500 px wide.
- Step 3: the Run tool window shows a tab named after the configuration;
  the first line is the command line (`<uv> run <venv python> <script>
  <parameters>` for an environment of a uv project with "Run with `uv
  run`" on or unset); the environment has the env files' variables, the
  configuration's over them, `PYTHONUNBUFFERED=1`, the content and source
  roots on `PYTHONPATH` when those options are on, and the system
  environment only when "Include system environment variables" is on;
  the working directory is the configured one, else the project folder.
- Step 4: each file gets a temporary configuration named after it,
  selected and run; the sixth removes the oldest one that is neither
  recent nor pinned; the same file again reuses its configuration.
- Step 5: Save Configuration makes it permanent (it stays personal); Pin
  puts it under "Pinned"; Delete asks "Are you sure you want to delete
  …?" ("Don't ask again"), Yes deletes it, a shared one's file too.
- Step 6: the dialog "Run/Debug Configurations" shows the tree type →
  folder → configuration in list order, the toolbar Add New Configuration
  (a list of types), Remove Configuration, Copy Configuration, Save
  Configuration, Create New Folder, Sort Configurations, the name with
  "Store as project file", the type's fields and Modify options (groups
  Operating System, Python, Debug, Before Launch, Allow multiple
  instances; options at their default are hidden from the form), an error
  line ("Please specify a script name"), Cancel, Apply, OK. "Store as
  project file" with personal values asks whether the values stay
  personal; the shared file lists those variables by name only. Cancel
  drops every change since the last Apply.
- Step 7: the pytest configuration is listed under "Python tests", read-
  only in the dialog with its decoded options; Run is disabled and says
  the type is not supported yet. The configuration with the missing
  interpreter runs with the project's; the status bar names both.
- Step 8: no second import; the selection is kept. Without the personal
  state only the per-user configurations are imported again; the shared
  ones are not duplicated.

## Commands
`run.configurations`, `run.configuration.select {"name": ..., "type":
...}`, `run.configuration.run {"name": ...}`, `run.select-current-file`,
`run.file`, `run.current-file`, `run.configurations.toggle-all`,
`run.configuration.save-temporary`, `run.configuration.pin`,
`run.configuration.delete`, `run.configurations.edit`,
`run.configurations.import-reference`, `run.configurations.dialog.*`
(`select`, `next`, `previous`, `add`, `remove`, `copy`,
`save-temporary`, `new-folder`, `sort`, `set {"field", "value"}`,
`toggle-option {"option"}`, `modify-options`, `apply`, `ok`, `cancel`),
`dialog.choose`, `run.run`, `run.state` (`configurations`, `widget`,
`menu`, `edit_dialog`), `workbench.state` (`notice`, `message`,
`dialog`).

## Source
https://www.jetbrains.com/help/pycharm/run-debug-configuration.html;
IntelliJ Platform Community (Apache-2.0) `RunManagerImpl`,
`RunnerAndConfigurationSettingsImpl`, `RunConfigurationListManagerHelper`,
`RunConfigurationSchemeManager`, `RCInArbitraryFileManager`,
`RunToolbarPopup`, `RedesignedRunWidget`, `RunConfigurationsComboBoxAction`,
`ChooseRunConfigurationManager`, `RunConfigurable`,
`EditConfigurationsDialog`; Python plugin `AbstractPythonRunConfiguration`,
`PythonRunConfiguration`, `PythonScriptCommandLineState`,
`PythonCommandLineState`, `PythonConfigurationFragmentedEditor`,
`UvRunToolProvider`, `LegacyUvRunConfigurationConverter`; Shell Script
plugin `ShRunConfiguration`.
