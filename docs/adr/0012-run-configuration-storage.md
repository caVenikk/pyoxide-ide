# 0012. Run configuration storage

- Status: accepted
- Date: 2026-10-15

## Context
Run configurations are named, repeatable launches: a server script with
an env file, a module with parameters, a shell script. The reference IDE
keeps them in two places (`RunManagerImpl`, `RunConfigurationSchemeManager`,
`RCInArbitraryFileManager`):

- per-user configurations in `.idea/workspace.xml`, component `RunManager`,
  together with the selected configuration (`selected`, a unique id
  `"<type display name>.<name>"`), a custom order (`<list>`), the recently
  used temporary configurations (`<recent_temporary>`) and templates
  (`default="true"`); the last executor of each configuration is a
  `<unique id>.executor` property of `PropertiesComponent`;
- shared configurations ("Store as project file") one per file in
  `.idea/runConfigurations/`, or in `*.run.xml` files anywhere in the
  project (`.run/` by default), each wrapped in
  `<component name="ProjectRunConfigurationManager">`.

Pinned configurations and the start history are in a per-product
workspace file outside `.idea`. Paths are written with path variables
(`$PROJECT_DIR$`, `$PROJECT_DIR$/..`, `$MODULE_DIR$`, `$USER_HOME$`, user
path variables). Environment variables of a configuration are written in
the same file as the configuration, so the values of a shared one are
committed with it.

People who use the reference IDE and Pyoxide on the same project already
have configurations in `.idea`. ADR 0008 and ADR 0009 set the pattern:
Pyoxide keeps its own settings in `.oxide/`, shared ones in a committed
file and personal ones in `.oxide/workspace.toml`, imports the reference's
once and never writes `.idea`. Environment values may be secrets
(tokens, passwords) and must not end up in a committed file by accident.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
platform/execution-impl/src/com/intellij/execution/impl/RunManagerImpl.kt,
RunnerAndConfigurationSettingsImpl.kt, RunConfigurationListManagerHelper.kt,
RunConfigurationSchemeManager.kt, RCInArbitraryFileManager.kt,
RunConfigurationStorageUi.java; .../execution/ui/RunToolbarPopup.kt,
RedesignedRunWidget.kt; .../execution/envFile/EnvFileParser.kt;
platform/util/src/com/intellij/util/execution/ParametersListUtil.java;
python/src/com/jetbrains/python/run/AbstractPythonRunConfiguration.java,
PythonRunConfiguration.java, PythonScriptCommandLineState.java,
PythonCommandLineState.java, LegacyUvRunConfigurationConverter.kt;
python/src/com/jetbrains/python/sdk/uv/run/UvRunConfiguration.kt;
plugins/sh/core/src/com/intellij/sh/run/ShRunConfiguration.java.

## Options considered
1. Read and write the reference's files. Both IDEs would see the same
   configurations, but the format belongs to the reference, which rewrites
   the files itself; two IDEs writing `workspace.xml` lose each other's
   changes.
2. All configurations in `.oxide/project.toml` and `.oxide/workspace.toml`.
   Few files, but every change to a shared configuration touches one file
   that the whole team edits, and merges conflict.
3. One TOML file per shared configuration in `.oxide/run/`, everything
   personal in the `[run]` table of `.oxide/workspace.toml`, environment
   values split by a per-variable flag, and a one-time import of the
   reference's configurations per storage class.

## Decision
Option 3. The model, storage and import are the platform crate
`run-config` (no UI toolkit); products register their types (ADR 0005).

### Shared configurations
- One file per configuration in `.oxide/run/`, meant to be committed, as
  the reference's `.run/` keeps merges small: `<name>.toml`, characters
  file names cannot hold replaced by `_`, ` (2)` and so on when the name
  is taken (case ignored).
- The file has a `version` (now 1), `name`, `type` and the common
  settings, and an `[options]` table owned by the type:

  ```toml
  # Pyoxide run configuration, shared with the project. Paths are relative to the project folder.
  version = 1
  name = "server"
  type = "python"
  folder = "Servers"
  env = [
      { name = "PYTHONUNBUFFERED", value = "1" },
      { name = "API_TOKEN" },
  ]

  [options]
  script = "app/server.py"
  env-files = [".env"]
  ```

- The conventions of ADR 0008 hold: keys this version does not know
  survive a rewrite (unchanged keys keep their comments), a file of a
  newer version is read as far as it is understood and never rewritten,
  writes replace a file at once, and a file is written only when its
  content changed.

### Personal state
- The `[run]` table of the personal project state (`.oxide/workspace.toml`,
  ADR 0009, ignored by Git) holds the personal configurations
  (`[[run.configurations]]`, everything imported from `workspace.xml`, the
  temporary ones), the personal values of shared configurations'
  variables (`[[run.shared-env]]`), the selected configuration, the custom
  order, the recent temporary, pinned and history lists and the last
  executor of each configuration. A configuration is identified by its
  type and name (`{ type = "python", name = "server" }`); names are unique
  within a type, as in the reference.

### Environment values
- Each variable has a `shared` flag. A shared configuration's file lists
  every variable by name and has a value only for shared ones; the values
  of the others stay in the personal state. A variable with no value on
  this machine is "not set on this machine" and is not passed to the
  process. All values of a personal configuration are personal.
- Imported variables of shared reference configurations are shared (they
  were committed there already); so are the type's defaults such as
  `PYTHONUNBUFFERED=1`. A variable added to a shared configuration is
  personal by default.
- Values never appear in logs, errors or `Debug` output (`EnvVar` and the
  kept XML of unsupported configurations are redacted). Env files are
  stored as paths and read at launch: their variables first, then the
  configuration's over them.

### Paths
- Stored relative to the project root with `/` on every OS; the project
  folder itself is `.`, its parent `..`. On import `$PROJECT_DIR$` and
  `$MODULE_DIR$` become relative paths; `$USER_HOME$` and user path
  variables are kept as written and flagged in the editor. At launch
  relative paths resolve against the project, and `$PROJECT_DIR$`,
  `$MODULE_DIR$` and `$USER_HOME$` are expanded (also in parameters);
  other macros (`$Prompt$`) stay as they are.

### Import from the reference
- Runs off the UI thread when a project opens, once per storage class,
  with its own markers, because `.oxide/` may exist already from the
  exclusions import:
  - `imported-from-idea = true` in the `[run]` table of the shared
    settings (`.oxide/project.toml`) after shared configurations were
    imported, so a teammate who clones the project does not import them
    again. The exclusions import of ADR 0008 runs first, since it happens
    only while `project.toml` does not exist;
  - the same key in the personal `[run]` table after the per-user
    configurations of `workspace.xml` were imported, so each person's are
    imported once on their machine.
  A marker is written only when the import brought something; until then
  the next open looks again (`.oxide/` is not created for nothing).
- Reads `RunManager` of `.idea/workspace.xml` (configurations, `selected`,
  `<list>`, `<recent_temporary>`), the executors of `PropertiesComponent`,
  `.idea/runConfigurations/*.xml` and the `*.run.xml` files in the
  project's shown and not excluded folders. Shared configurations are
  added first and keep their name; a configuration whose type has the
  name already gets ` (1)`, ` (2)`, … (`RunManager.suggestUniqueName`) and
  is listed in the notification "Imported N run configurations (M kept
  read-only)". Templates are skipped. `.idea` is never written.
- Configurations of types no product converts (pytest, Compound, Docker,
  Django, FastAPI, Flask, Jupyter, unknown types) are kept read-only with
  their `<configuration>` element byte for byte, in the storage class
  they came from, as type `idea:<reference type id>`. They are listed with
  their type's name and cannot run; Delete works.
- The reference's standalone `uv run` type becomes a Python configuration
  with `uv run` on, named `uv run <name>`, as the reference converts it
  itself in later versions.
- `run.configurations.import-reference` repeats the import on request and
  adds only configurations whose type and name do not exist yet.
- Pinned configurations and the history are not in `.idea` and are not
  imported.

### Order and selection
- The list order is the reference's: by type name (unknown types last),
  by folder (natural order, or the custom order; no folder last),
  permanent before temporary, then by name in natural order or the custom
  order. When nothing is selected after loading, the first configuration
  is selected (`selectAnyConfiguration`); Current File is "no selection".
- At most 5 temporary configurations are kept (a setting); the oldest
  that are neither recently used nor pinned go first.

## Consequences
- A project with shared configurations gets `.oxide/run/` and a `[run]`
  table in `.oxide/project.toml`; personal ones live only in the ignored
  `workspace.toml`, so a clone has the shared configurations and none of
  the personal values.
- There is no two-way sync: configurations changed in the reference after
  the import do not reach Pyoxide, and Pyoxide's do not reach the
  reference.
- A variable that becomes shared writes its value into a committed file;
  turning on "Store as project file" asks about the personal values
  first.
- Comments of a shared file are kept only on the keys that did not change
  in a rewrite.
- Environment values are plain text in the personal state; storing them
  in the OS credential store (the `secrets` crate) can follow.
