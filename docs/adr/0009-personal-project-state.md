# 0009. Personal project state

- Status: accepted
- Date: 2026-10-13

## Context
ADR 0008 gave Pyoxide a project folder, `.oxide/`, with shared settings in
`.oxide/project.toml` that are meant to be committed. It left open where
personal state goes: what belongs to one person on one machine, such as the
chosen Python interpreter (an absolute or machine-specific path), and later
the open tabs and the window layout. Committing such state makes every
member of a team overwrite the others' choices.

The reference IDE keeps the same split in its `.idea` folder:

- Shared files (`misc.xml`, the module files) are usually committed. They
  name the project's interpreter by its SDK name; the name is resolved
  through the IDE's own SDK table (`jdk.table.xml` in the IDE's
  configuration), which holds the paths per person.
- `workspace.xml` holds personal state and is not meant to be committed.
  When a directory-based project is created or opened, the Git integration
  writes `.idea/.gitignore` once (`GitIgnoreInStoreDirGenerator`), only if
  the folder has no `.gitignore` yet and is not already tracked by Git; it
  never rewrites an existing one. The file lists the default ignored files
  (`ChangeListManagerImpl.DefaultIgnoredFileProvider`) under their group
  comment:

  ```
  # Default ignored files
  /shelf/
  /workspace.xml
  ```

Pyoxide has no table of configured interpreters yet (it comes with the
interpreter settings, #75); the interpreter widget offers the environments
found in the project and stores the chosen one by path.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
plugins/git4idea/backend/src/ignore/GitIgnoreInStoreDirGenerator.kt and
GitIgnoredFileContentProvider.kt, platform/vcs-impl/src/com/intellij/
openapi/vcs/changes/ChangeListManagerImpl.java,
ignore/IgnoredFileGeneratorImpl.java and ignore/psi/util/IgnoreFileUtil.kt;
jps/model-serialization/src/org/jetbrains/jps/model/serialization/
JpsProjectConfigurationLoading.java, library/JpsSdkTableSerializer.java
and module/JpsModuleRootModelSerializer.java (where a project names its
SDK and how the SDK table is written).

## Options considered
1. Personal values in `.oxide/project.toml`. One file, but every choice
   becomes a change to a committed file.
2. Personal state in the application data folder, keyed by the project
   path. Never committed by accident, but lost or orphaned when the
   project moves, and invisible next to the project it belongs to.
3. A second file in the project folder, `.oxide/workspace.toml`, ignored
   by Git through `.oxide/.gitignore`, as the reference does with
   `workspace.xml`.

## Decision
Option 3.

- `.oxide/workspace.toml` holds personal project state. It follows the
  conventions of `project.toml`: TOML with a `version` (now 1), keys and
  comments Pyoxide does not know survive a rewrite, a file of a newer
  version is read as far as it is understood and never rewritten, writes
  replace the file at once (a temporary file and a rename), and the file
  is created only when there is a value to store.
- The store in the platform crate `project` is language-agnostic:
  `project::workspace::load_value` and `save_value` read and write a
  string by table and key (`None` removes the key). Products choose their
  tables; the Python product writes `[python] interpreter`.

  ```toml
  # Pyoxide personal project state (this person, this machine). Not meant to be committed.
  version = 1

  [python]
  interpreter = ".venv/bin/python"
  ```

- Paths inside the project are stored relative to the project root with
  `/` separators; paths outside it are stored as they are.
- When Pyoxide creates `workspace.toml`, it also writes `.oxide/.gitignore`
  unless that file exists (an existing one, even an empty one, is never
  rewritten):

  ```
  # Default ignored files
  /workspace.toml
  ```

  It is written together with the personal file, so a person who deletes
  the `.gitignore` does not get it back while the personal file exists.
  Pyoxide has no version control integration yet, so it does not check, as
  the reference does, whether the folder is already tracked by Git.
- The one-time import of the reference's exclusions (ADR 0008) now depends
  on the shared settings file rather than on the folder: a project that
  has only personal state still reads `.idea` until `project.toml` exists.
  Before this change the folder held nothing but `project.toml`, so the
  rule is unchanged for existing projects.
- The interpreter is personal for now because Pyoxide stores it by path.
  The reference shares a name and keeps the path per person; once
  Pyoxide has named, configured interpreters (#75), the shared name can
  move to `project.toml` while the paths stay personal.
- The reference's interpreter of a project is imported once, like its
  exclusions (ADR 0008), and `.idea` is never written: while the personal
  state holds no interpreter, Pyoxide reads the SDK the project names (the
  first module's `orderEntry type="jdk"`, else `ProjectRootManager` in
  `.idea/misc.xml`) and looks it up in the SDK table
  (`options/jdk.table.xml`) of the newest PyCharm configuration directory
  that has it. When its interpreter exists, it becomes the choice, and its
  SDK name is kept for it, so the widget shows the name the person knows
  from the reference:

  ```toml
  [python]
  interpreter = ".venv/bin/python"
  sdk-name = "uv (demo)"
  sdk-home = ".venv/bin/python3.13"
  ```

## Consequences
- A project with personal state has a `.oxide/` folder with two files
  that Git ignores and, if the team commits `.oxide/project.toml`, a
  third that it does not.
- Personal state does not travel with a clone or between machines, as
  intended. Choosing the interpreter in one checkout does not change it
  in another.
- The `.gitignore` rule covers Git only; other version control systems
  see the personal file as a new file.
- Open tabs, the window layout and other personal state can be added as
  new tables without a format change. Importing run configurations from
  `.idea` is not part of this decision.
- A later change of the interpreter in the reference does not reach
  Pyoxide once the import has happened, as with exclusions.
