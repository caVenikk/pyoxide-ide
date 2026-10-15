# Run configurations and their import from the reference IDE

## Steps
1. Open a project the reference IDE has used, with per-user
   configurations in `.idea/workspace.xml` (a Python script with an env
   file, a module run, one in a folder, a temporary one, a pytest one),
   a shared one in `.idea/runConfigurations/` and one in `.run/*.run.xml`;
   one of the per-user ones has the name of the shared one.
2. Open the run widget's dropdown.
3. Run the selected configuration with Ctrl+R (macOS) or Shift+F10.
4. Select another configuration in the dropdown, then "Current File".
5. Select the pytest configuration.
6. Reopen the project; delete `.oxide/workspace.toml` and reopen it.

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
  separator after each type, folders as submenus with the folder icon,
  types Pyoxide does not support last when unknown; then "Current File",
  a separator, "Edit Configurations…". With 5 or more and a history:
  "Pinned" (when something is pinned), "Recent Configurations", "All
  Configurations  N" that shows the rest. The selected row is
  preselected; typing filters.
- Step 3: the Run tool window shows a tab named after the configuration;
  the first line is the command line (`<uv> run <venv python> <script>
  <parameters>` for an environment of a uv project with "Run with `uv
  run`" on or unset); the environment has the env files' variables, the
  configuration's over them, `PYTHONUNBUFFERED=1`, the content and source
  roots on `PYTHONPATH` when those options are on, and the system
  environment only when "Include system environment variables" is on;
  the working directory is the configured one, else the project folder.
  An interpreter the configuration names that is not found is replaced
  by the project's, with a message in the status bar.
- Step 4: the widget follows the selection; with "Current File" Run runs
  the file in the editor again.
- Step 5: Run is disabled; its tooltip says the type is not supported
  yet.
- Step 6: no second import; the selection is kept. Without the personal
  state only the per-user configurations are imported again; the shared
  ones are not duplicated.

## Commands
`run.configurations`, `run.configuration.select {"name": ..., "type":
...}`, `run.configuration.run {"name": ...}`, `run.select-current-file`,
`run.configurations.toggle-all`, `run.configurations.import-reference`,
`run.run`, `run.state` (`configurations`, `widget`, `menu`),
`workbench.state` (`notice`).

## Source
https://www.jetbrains.com/help/pycharm/run-debug-configuration.html;
IntelliJ Platform Community (Apache-2.0) `RunManagerImpl`,
`RunnerAndConfigurationSettingsImpl`, `RunConfigurationListManagerHelper`,
`RunConfigurationSchemeManager`, `RCInArbitraryFileManager`,
`RunToolbarPopup`, `RedesignedRunWidget`; Python plugin
`AbstractPythonRunConfiguration`, `PythonRunConfiguration`,
`PythonScriptCommandLineState`, `PythonCommandLineState`,
`UvRunToolProvider`, `LegacyUvRunConfigurationConverter`; Shell Script
plugin `ShRunConfiguration`.
