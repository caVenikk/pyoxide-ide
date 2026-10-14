# Tool discovery, settings and the type engine

## Steps
1. Open a project whose `.idea/pyLspTools.xml` has `PyToolsState` with
   ruff enabled and Lookup Path; open a `.py` file.
2. Open a project whose file has the older form
   (`RuffConfiguration` with `enabled`, `executableDiscoveryMode`).
3. Lookup Sdk with ruff installed in the project interpreter; then with
   ruff only on PATH; then nowhere, with and without uv.
4. Click the type engine widget (`Types: ...`).
5. Change the project interpreter.

## Expected
- Step 1: ruff starts as `ruff server` found on PATH.
- Step 2: the same settings apply (the older format is migrated once).
- Step 3: Sdk tries the interpreter's `bin` (`Scripts` on Windows) first,
  then a custom path or PATH and per-user tool folders, then `uvx ruff`;
  with neither the tool nor uv nothing starts and no balloon is shown
  (Pyoxide: see the issue for the missing-tool notice).
- Step 4: popup "Type Engine" with the engines, a check mark on the
  current one, and "Type Engine Settings…".
- Step 5: type servers restart with the new interpreter; open files are
  analysed again.

## Commands
`python.tools.state`, `python.type-engine.select {"name": ...}`.

## Source
https://www.jetbrains.com/help/pycharm/lsp-tools.html,
https://www.jetbrains.com/help/pycharm/python-type-engine.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `PyToolsState`,
`ExecutableDiscoveryMode`, `PyToolExt.getExecutableWithBaseArgs`,
`PyTypeEngineProjectSettings`, `PyTypeEngineStatusBarWidget`,
`RestartLspServersListener`.
