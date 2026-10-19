# Python environments

How Pyoxide finds Python interpreters, reads a project and changes its
environment. The code is the product crate `crates/python/py-env` (library, the
`env.*` commands) and `crates/python/pyoxide-cli` (the `pyoxide` binary); both
sit in the Python product layer of [ADR 0005](adr/0005-platform-and-language-products.md)
and use the platform command registry ([ADR 0001](adr/0001-command-registry.md)).

Pyoxide does not reimplement a package manager. [uv](https://docs.astral.sh/uv/)
does the work (virtual environments, projects, locking, installing, Python
downloads); `python -m pip` is the fallback for listing and installing when uv is
missing. Everything runs as a child process behind a typed function.

## Rules for every external process
- Started with tokio, never on the caller's thread; stdin is closed.
- Bounded by a timeout (20 s for queries, minutes for installs; `RunContext::timeout`
  overrides it) and by a cancellation token. Dropping the future kills the child
  as well, which is how a command call is abandoned.
- uv's progress and pip's report arrive line by line through a progress sink
  (`RunContext::progress`). The last lines are also returned in the result's `log`.
- Output is captured with a bound: the first 100 and last 200 lines of stderr,
  32 MiB of stdout. Credentials in URLs (`user:password@`) are replaced by `***@`
  in progress lines, messages and logs.
- On Windows no console window is opened.
- uv is run with `NO_COLOR=1` and `UV_NO_PROGRESS=1`; `UvConfig` can add
  `UV_OFFLINE`, `UV_PYTHON_DOWNLOADS=never`, `UV_NO_CONFIG`, `UV_CACHE_DIR` and
  `UV_PYTHON_INSTALL_DIR`. Nothing else of uv's own configuration is touched.

## Finding tools
uv is searched in `$PYOXIDE_UV`, then `PATH`, then the places installers use
(`~/.local/bin`, `~/.cargo/bin`, Homebrew and `/usr/local/bin` on Unix,
`%LOCALAPPDATA%\Programs\uv` on Windows). A GUI application does not inherit the
shell's `PATH`, which is why the extra places matter. Poetry and conda are found
the same way (`CONDA_EXE` first for conda).

## Interpreter discovery
`discover` (`env.interpreters.list`) gathers candidate executables from several
sources at the same time, removes duplicates and probes each remaining one.

| Source | How | When |
|---|---|---|
| uv | `uv python list --only-installed --output-format json` | uv installed |
| PATH | files named `python`, `python3`, `python3.N` in the `PATH` directories and the tool directories above | always |
| Project environments | `.venv` and `venv` in the project directory and its parents, `UV_PROJECT_ENVIRONMENT`, `VIRTUAL_ENV` | `.venv`, `venv` and `UV_PROJECT_ENVIRONMENT` when a project directory is given; `VIRTUAL_ENV` always |
| Poetry | `poetry env info --path` in the project | Poetry installed and the project has `[tool.poetry]` or `poetry.lock` |
| conda | `conda env list --json`; without conda, the paths in `~/.conda/environments.txt`; `CONDA_PREFIX` | always |
| pyenv | `$PYENV_ROOT/versions/*` (default `~/.pyenv/versions`; pyenv-win layout on Windows) | the directory exists |
| Python launcher | `py -0p` | Windows, `py` installed |

**Duplicates.** Two candidates are the same interpreter when their canonical paths
are equal, so `python3`, `python3.13` and a uv shim pointing at one file give one
entry (the best-ranked source wins: project environments, then conda, uv, pyenv,
PATH; ties keep the most specific name). A virtual environment is identified by its
directory instead, because its `python` is a link to the base interpreter.

**Probe.** Each interpreter is run once as `python -I -S -c <script>`. The script
prints one JSON line: version (with release level), implementation, `sys.prefix`,
`sys.base_prefix`, machine and whether it is a free-threaded build. Isolated mode
and no `site` keep user files out and the start fast. At most eight probes run at
once, each limited to ten seconds. A candidate that fails (a broken link, Python 2,
a Windows Store stub) is listed under `skipped` with the reason; it is not an error.

**Result.** Per interpreter: path, version, implementation, kind, `is_venv`,
`prefix`, `base_prefix`, `venv_root`, architecture, free-threaded flag and the
source it was found through. The kind is decided from what the probe reports and
where the file lives:

| Kind | Rule |
|---|---|
| `venv` | `sys.prefix` differs from `sys.base_prefix` |
| `poetry` | a venv reported by Poetry |
| `conda` | found by conda, or the prefix contains `conda-meta` |
| `uv` | the real path is inside uv's Python directory (`uv python dir`, or `UV_PYTHON_INSTALL_DIR`) |
| `pyenv` | found in the pyenv versions directory, or under a `.pyenv` directory |
| `system` | everything else |

Results are sorted: environments of the project first, installations last, newest
version first within a kind. A callback receives each interpreter as soon as it is
probed, for incremental lists.

**macOS.** `/usr/bin/python3` is a stub that opens the Command Line Tools installer
when they are missing. It is only probed when `xcode-select -p` succeeds.

### What degrades
The report always lists the tools (`uv`, `poetry`, `conda`, `pyenv`, `py` on Windows)
with `installed`, path, version for uv and a note on what is lost.

| Missing | Effect |
|---|---|
| uv | no uv-managed interpreters from `uv python list` (they are still found on `PATH` when linked there); packages are listed and installed with pip; `env.venv.create`, `env.sync`, `env.lock`, `env.python.install`, project edits and the dependency tree report `env.tool_missing` |
| Poetry | the project's Poetry environment is not listed (a `.venv` inside the project still is) |
| conda | environments come from `~/.conda/environments.txt` only |
| pyenv | nothing; the versions directory is simply absent |
| a working Python | the list is empty; uv can still download one (`env.python.install`) |

## Project model
`load_project` (`env.project.info`) reads what a project declares, without running
anything. The path may be a directory inside the project, its `pyproject.toml` or a
`.py` script. The project root is the nearest directory upwards with a
`pyproject.toml`.

- `pyproject.toml`: `[project]` (name, version, description, `requires-python`,
  `dependencies`, `optional-dependencies`, `dynamic`), `[dependency-groups]`
  (PEP 735, with `include-group`; `PyProject::group_requirements` expands them and
  survives cycles), `[build-system] build-backend`, `[tool.uv]` (dev-dependencies,
  package, managed, default-groups, workspace members and excludes, names under
  `sources`), and the presence of `[tool.poetry]` / `[tool.pdm]`. A field of the
  wrong type becomes a warning; a TOML syntax error is reported in `problems`.
- `uv.lock`: `requires-python` and the locked packages with version and source kind.
- Requirements files: `requirements*.txt` in the root and `*.txt` in `requirements/`;
  requirements, `-r`/`-c` includes, line continuations and comments.
- `.python-version`: the first meaningful line.
- The project environment: `UV_PROJECT_ENVIRONMENT`, else `.venv` or `venv` in the
  project or a parent.
- PEP 723 inline metadata of a script: `requires-python`, `dependencies` and
  `[tool.uv]` from the `# /// script` block. Several script blocks or invalid TOML
  inside one are errors, as the specification demands.
- Managers: uv (lock file or `[tool.uv]`), Poetry, PDM, Pipenv, conda
  (`environment.yml`) and pip (requirements files) are detected from these files.
- `parse_requirement` splits a PEP 508 string into name (normalized per PEP 503),
  extras, specifier, marker and URL.

## Environments on disk
`py_env::venv` reads environments without starting Python, cheap enough to
run whenever a project opens:

- `PyvenvCfg`: the `key = value` lines of `pyvenv.cfg`. The version is
  `version_info` (uv, virtualenv; virtualenv's `3.12.1.final.0` becomes
  `3.12.1`) or `version` (the `venv` module); a `uv` key marks an
  environment uv created.
- `inspect(dir)`: a folder with an interpreter (`bin/python`,
  `Scripts\python.exe`, or `python.exe` of a conda prefix on Windows) is a
  virtual environment (`pyvenv.cfg`), a conda environment (`conda-meta`,
  the version from its `python-X.Y.Z-*.json` record) or an installation.
- `project_environments(root, UV_PROJECT_ENVIRONMENT)`: the environments of
  a project as the reference IDE finds them when a folder opens
  (`VirtualEnvReader.findVenvsInDir`): `UV_PROJECT_ENVIRONMENT` first, then
  every folder directly in the project root that holds an interpreter, by
  name, then the project environment of a parent folder; each once.

The IDE's interpreter widget lists these environments and selects the
project environment (`find_project_venv`) unless the person chose another
one, which is kept in the personal project state (ADR 0009). uv may write a
version without the patch level into `pyvenv.cfg` (`version_info = 3.14`);
the widget then asks the interpreter
(`python --version`, in the background, at most 5 s, remembered while the
file is unchanged), as the reference shows the full version.

## Operations
Each is a typed function with a request struct; the command line it runs is
deterministic and unit-tested.

| Function | Command run |
|---|---|
| `Uv::venv` | `uv venv [--python X] [--seed] [--clear] [--system-site-packages] <path>` |
| `Uv::init` | `uv init [--name N] [--lib\|--package\|--bare\|--script] [--python X] --vcs none <path>` (version control is the IDE's decision; `vcs: true` lets uv run `git init`) |
| `Uv::sync` | `uv sync [--extra E] [--all-extras] [--group G] [--all-groups] [--no-dev] [--frozen] [--locked]` in the project |
| `Uv::lock` | `uv lock [--upgrade] [--upgrade-package P]` |
| `Uv::add` / `Uv::remove` | `uv add\|remove [--group G \| --optional E \| --dev] [--script S] [--no-sync] [--frozen] -- <packages>` |
| `Uv::tree` | `uv tree [--depth N] [--outdated] [--no-dev] [--frozen] [--package P]`; the output is parsed into nodes (name, version, notes such as `extra: x`, children) and kept as text |
| `Uv::pip_list` | `uv pip list --python <env> --format json [--outdated]` |
| `Uv::pip_install` / `pip_uninstall` | `uv pip install\|uninstall --python <env> ...` |
| `Uv::python_install` | `uv python install [--reinstall] <versions>` |
| `Uv::python_list`, `python_dir`, `version` | queries used by discovery |
| `Pip::list` / `install` / `uninstall` | `python -m pip list --format json [--outdated]`, `install [--upgrade] [--requirement F] -- <pkgs>`, `uninstall --yes -- <pkgs>` |

`env.packages.add` and `remove` have two modes. Without `venv` they edit the
project (`uv add`/`uv remove`, which lock and sync unless told not to). With `venv`
they install into that environment (`uv pip`, or pip without uv) and the project
options (`group`, `optional`, `dev`, `script`, `no_sync`, `frozen`) are rejected.
`env.packages.list` finds the environment from `venv`, else the project's
environment, else `VIRTUAL_ENV`, and reports which `backend` answered.

### Errors
`EnvError` has a `kind`, a short readable `message`, the `tool`, the `exit_code` and
a condensed `detail` (the tool's error output without the version lists a resolver
prints). The kind is derived from the tool's output:

| Kind | Command error code | Typical cause |
|---|---|---|
| `tool_missing` | `env.tool_missing` | uv, pip or Python not installed; says what to do |
| `no_interpreter` | `env.no_interpreter` | unknown Python version, no download for it, no environment for the project |
| `no_project` | `env.no_project` | no `pyproject.toml` |
| `resolution` | `env.resolution` | the resolver found no consistent versions |
| `network` | `env.network` | index or download unreachable; also a resolution that failed because uv was offline |
| `package_not_found` | `env.package_not_found` | not on the index, or not in the project's dependencies |
| `already_exists` | `env.already_exists` | environment or project exists |
| `parse` | `env.parse` | unreadable project file or tool output |
| `timeout`, `cancelled`, `io`, `failed` | `env.timeout`, `env.cancelled`, `env.io`, `env.failed` | |
| `invalid_input` | `invalid_args` | bad arguments, such as a malformed package name |

## Package lookup
`PackageIndex::project(name)` returns summary, latest version, license, URLs and all
versions (newest first, with yanked and pre-release flags and `requires-python`).
`PyPi` implements it over `https://pypi.org/pypi/<name>/json` (names normalized per
PEP 503 and validated before any request). The blocking HTTP client runs on the
blocking pool with a 15 s limit, honours `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` and
trusts the operating system's certificate store. `StaticIndex` is the in-memory
implementation tests use. Versions are ordered by an own PEP 440 comparison
(`version::Version`).

## Commands
`py_env::register` adds these to a registry; `pyoxide commands` lists them together
with the platform's `db.*` commands. Results carry the tool's report in `log`
because the registry has no streaming yet.

| Command | Arguments (JSON) |
|---|---|
| `env.interpreters.list` | `project?` |
| `env.project.info` | `path?` |
| `env.project.init` | `path`, `name?`, `kind?` (`app`, `lib`, `package`, `bare`, `script`), `python?`, `build_backend?`, `no_readme?`, `no_pin_python?`, `vcs?` |
| `env.project.tree` | `project?`, `depth?`, `outdated?`, `no_dev?`, `frozen?`, `package?` |
| `env.venv.create` | `path`, `python?`, `seed?`, `clear?`, `system_site_packages?`, `prompt?` |
| `env.packages.list` | `venv?`, `project?`, `outdated?` |
| `env.packages.add` | `packages`, `project?`, `venv?`, `group?`, `optional?`, `dev?`, `script?`, `no_sync?`, `frozen?`, `requirements?`, `upgrade?` |
| `env.packages.remove` | `packages`, `project?`, `venv?`, `group?`, `optional?`, `dev?`, `script?`, `no_sync?`, `frozen?` |
| `env.sync` | `project?`, `extras?`, `all_extras?`, `groups?`, `all_groups?`, `no_dev?`, `frozen?`, `locked?`, `python?` |
| `env.lock` | `project?`, `upgrade?`, `upgrade_packages?`, `python?` |
| `env.python.install` | `versions`, `reinstall?` |
| `env.package.search` | `name` |

Paths default to the current directory; relative paths are resolved against it.

## The `pyoxide` command line
`pyoxide env <command>` calls the same typed functions as the commands above and
streams uv's progress to stderr (Ctrl-C cancels the running tool and exits with 130).
`--json` prints the result types as JSON and keeps stderr quiet; errors are printed
as `{code, message, tool, exit_code, detail}`.

```
pyoxide env interpreters [--project DIR]
pyoxide env info [PATH]
pyoxide env init [PATH] [--name N] [--lib|--package|--bare|--script] [--python V]
pyoxide env venv [PATH] [--python 3.12] [--seed] [--clear]
pyoxide env packages [--venv PATH] [--project DIR] [--outdated]
pyoxide env add <PACKAGE>... [--group G] [--optional E] [--dev] [--venv PATH] [-r FILE]
pyoxide env remove <PACKAGE>... [--group G] [--venv PATH]
pyoxide env sync | lock | tree [--project DIR] ...
pyoxide env python-install <VERSION>...
pyoxide env search <NAME> [--versions N]
pyoxide commands
pyoxide call <id> [<json>]
```

## Platform notes
- Paths: `bin/python` on Unix, `Scripts\python.exe` for venvs and `python.exe` for
  conda and pyenv-win on Windows; executables are looked up with `PATHEXT`.
- The `py` launcher is only used on Windows. Tests that create fake interpreters as
  shell scripts are Unix-only.
- Canonical paths are used only to compare; shown paths are the ones found.

## Tests
- Unit tests (always run, no network): command lines, parsers for uv's JSON and tree
  output, pyproject, `uv.lock`, requirements and PEP 723 samples, error
  classification, PEP 440 ordering, discovery over fake interpreters, a local HTTP
  server for the PyPI client.
- `crates/python/py-env/tests/it/uv_integration.rs` runs the real uv offline in
  temporary directories (venv, init, add and remove with `--frozen`, lock, sync, tree,
  `pip` install of a wheel built on the spot, typed failures). It skips itself without
  uv or an installed Python.
- `crates/python/pyoxide-cli/tests/it/cli.rs` runs the binary: `commands`, `call` and an
  offline project workflow.
- Ignored, need the network (`cargo test -p py-env --test it uv_integration:: -- --ignored`):
  adding and syncing `requests` from PyPI with the outdated check and the tree,
  `uv python install` into a temporary directory, the pip fallback on a seeded venv,
  and the live PyPI lookup.
