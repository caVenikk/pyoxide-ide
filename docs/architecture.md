# Architecture

## Principles
1. Rust everywhere in the application; external tools only as processes behind
   protocols (LSP, DAP, the Python interpreter).
2. Engine first: a pure-data model and a **command registry**; the UI is a thin
   layer. The UI, the CLI, a JSON control channel and agents call the same commands.
3. Nothing blocks the UI thread. Analysis and I/O are async, cancellable, incremental.
4. Traits at the boundaries: UI crates depend on traits (`db-core`), never on a
   concrete driver.

## Current crates
Platform (`crates/platform/`, language-agnostic):
- `commands`: the command registry (ADR 0001).
- `db-core`: `Driver`, `Connection` and `Cursor` traits, typed values,
  cancel/timeout policy, errors, introspection model and per-dialect SQL,
  shared driver building blocks (blocking-driver worker, value conversion,
  Oracle statement text, transaction state), text table renderer (ADR 0002).
- `db-oracle-thin`: pure Rust Oracle driver (`oracle-rs`, TNS protocol).
- `db-oracle-odpi`: Oracle via an installed Oracle Client (ODPI-C), fallback for
  environments the thin driver cannot reach (Kerberos, LDAP naming, native
  network encryption, ...). Client libraries are loaded at runtime.
- `db-postgres`, `db-sqlite`: PostgreSQL and SQLite drivers (ADR 0003).
- `db-testkit`: the conformance suite every database driver runs.
- `db-service`: connection profiles (ADR 0004), driver dispatch, diagnostics,
  import of PyCharm data sources, `db.*` commands.
- `db-cli`: `pyoxide-db`, a thin front-end over `db-service`.
- `secrets`: secret stores (OS credential store, KeePass, memory), their
  settings and the reader of a JetBrains IDE credential store for import.
- `settings-import`: PyCharm project settings (data sources, JDBC URLs).
- `text`: the text model (ADR 0007): rope buffer, selections, undoable
  transactions, caret movement, encoding and line separator of the file,
  reloading from disk as one undo step with carets kept through a line
  diff.
- `search`: the text search engine of Find / Replace in the editor and of
  Find in Files: plain and regular expression queries with case and whole
  word options, replacement templates, cancellable searches over text
  snapshots off the UI thread.
- `syntax`: tree-sitter parsing off the UI thread and highlight spans as
  theme text roles; grammars and queries come from the products.
- `project`: an opened folder, its lazily listed file tree with hidden and
  excluded entries, Pyoxide's project folder (`.oxide/`) with the shared
  project settings (exclusions, source roots, and values by dotted table
  for products) and the one-time import of the reference IDE's exclusions
  and source folders (ADR 0008, 0013), the folders a product sees as
  packages or modules,
  the personal project state (ADR 0009), reading and writing documents
  with the stamps (size, modification time) that tell external changes
  from the application's own writes, what paths reported by the file
  watcher are now on disk, and the file operations of the project tool
  window (`fs_ops`: the reference's checks of new names, create from
  templates and packages, rename, copy and move with their conflicts,
  delete to the system trash; never outside the project).
- `file-watcher`: the operating system's change notifications (`notify`:
  FSEvents, inotify, ReadDirectoryChangesW) for the project folder,
  gathered into batches of changed paths with a hint each and a rescan
  flag when events were lost.
- `vcs`: the version control model without a backend or a toolkit (ADR
  0016): file statuses, changes and the status map of a repository
  (ignored folders count for everything below them, an unsaved document
  of a clean tracked file is modified), line ranges of a document against
  its base text (`imara-diff`, git's histogram algorithm) shifted on
  edits, `HEAD`, the operation in progress, branches and the `Backend`
  trait UI crates depend on.
- `git`: Git through the git executable (ADR 0016): lookup and version
  check, a runner with per-repository read/write locks and cancellation
  (`GIT_OPTIONAL_LOCKS=0` for reads), porcelain and `for-each-ref`
  parsers, readers of `HEAD`, refs, `packed-refs`, the config and the
  state markers (reftable repositories ask git), repository detection at
  or above a folder, the status service fed by the file watcher (300 ms
  debounce, limited to the dirty paths when few, own writes suppressed),
  base texts through `cat-file --filters` cached by blob, the
  `.idea/vcs.xml` mapping, `git.status`, `git.branches`, `git.state`.
- `run-config`: run configurations without a toolkit (ADR 0012): the
  language-neutral model (common settings, environment variables with
  shared or personal values, options per type), the list with the
  selection, the reference's order, temporary ones, pinned ones and the
  history; storage in `.oxide/run/` (shared) and the personal project
  state; the one-time import of the reference IDE's configurations
  (`.idea/workspace.xml`, `.idea/runConfigurations`, `*.run.xml`) with
  importers per type; the forms the Run/Debug Configurations dialog
  draws (`TypeEditor`); the Shell Script type.
- `project-search`: Find in Files without a toolkit: the files of a scope
  that are project content (the project's hidden and excluded entries left
  out), file masks, a parallel cancellable search that streams the matching
  lines and stops at a limit (later passes skip the lines shown), the
  results as ordered rows, and replacing in files on disk.
- `name-match`: the reference IDE's name matcher (camel humps, word
  starts, wildcards, middle matches) and its matching degree, for Go to
  File, actions and speed search.
- `file-index`: the project's files and folders by name, built in the
  background with the project tree's rules and kept current, and Go to
  File's matching and ranking (`name:line:column` included).
- `editor`: the code editor entity and element (virtualized painting,
  carets, selections, clipboard, input method, highlighting, editing
  commands) and its find bar (Find / Replace in the file); diagnostic
  marks, a link highlight, hover events and geometry for popups (stored
  and reported; painting comes with the problems in the editor); soft
  wraps: the reference's wrap positions, the rows of every line with
  prefix sums, updated by edits and computed off the UI thread for large
  texts, carets and painting by visual rows.
- `code-intel`: the language-agnostic model and `CodeIntel` trait of code
  intelligence (ADR 0010): documents, positions as `text::Point`,
  diagnostics, completion items, documentation, signatures, locations,
  text and workspace edits, code actions, server status and events; an
  in-memory `FakeCodeIntel` (feature `fake`) for tests.
- `lsp-client`: the LSP client (ADR 0010): server specs, the product's
  `ServerCustomizer`, and `LspCodeIntel`, `CodeIntel` over several
  servers. A skeleton: servers are not started yet.
- `code-intel-ui`: the code intelligence of a main window: the product
  hook (`CodeIntelProvider`, `CodeIntelHook`), the `CodeIntelView` the
  workbench reports editors to, the `code.*` and `completion.*` commands.
  A skeleton: the commands do nothing yet.
- `themes`: theme model and tokens, default themes, import of PyCharm colour
  schemes and UI themes.
- `ui`: component layer on GPUI (ADR 0006, 0007): theme as GPUI colours and
  metrics, bundled fonts, colour icons rasterized at device scale, common
  components, context menus and list popups, dialog windows (a separate
  OS window modal to its parent: `dialog_window`, used by the Run/Debug
  Configurations dialog), the UI command index (`ui_commands!`,
  `command::invoke`, which runs commands in an open dialog).
- `workbench`: the main window (toolbar, stripes, tool windows, editor
  area, status bar; the tool window registry `tool_windows` puts each tool
  window in the left, bottom or right area, one shown per area with a size
  per area, and other crates register their views through
  `ToolWindowView`), a movable and resizable popup frame, the Find in Files
  and Replace in Files popup, Search Everywhere and Go to File with one
  file preview component (`file_preview`: editable in Find in Files,
  read-only in Search Everywhere, with the Soft-Wrap button), Recent
  Files), the project tool window's file operations (New
  with the product's kinds, Rename, Delete, Cut, Copy, Paste, Move, Copy
  Path and drag and drop, with their popups and dialogs; tabs, the file
  index, the navigation history and the language services follow), file
  types (associations by whole name, pattern and extension kept as data
  that later ones override, files of unknown names typed by their first
  bytes off the UI thread, file icons and icon rules), the
  navigation history (Back and Forward), keymaps (with double taps of a
  modifier and the mouse's back and forward buttons), application
  start-up, environment switches and in-app measurements; files changed
  outside the application (the file watcher, a look at the open files
  when the window gets the focus, Reload All from Disk, the File Cache
  Conflict dialog, saves that never write over a newer file); the
  project's Git repository with its statuses (`version_control.rs`).

Python product (`crates/python/`):
- `py-env`: interpreter and virtualenv discovery, project model
  (`pyproject.toml`, `uv.lock`, PEP 723), uv and pip operations, `env.*`
  commands.
- `pyoxide-cli`: `pyoxide`, the product's command line over the platform and
  Python command registries.
- `py-syntax`: the Python grammar and highlight query for `syntax`.
- `py-lsp`: Python language services (ADR 0010, 0011): the tools (ruff,
  ty, basedpyright), their lookup modes, settings and server specs, the
  import of the reference's tool settings. A skeleton.
- `pyoxide-ide`: `pyoxide-ide`, the product's GUI application: the
  workbench with Python file types. Run it with
  `cargo run --release -p pyoxide-ide [folder] [files]`.

The application shell and editor are described in ADR 0007.

## Platform and products
One repository, one Cargo workspace: a language-agnostic platform layer and
one product layer per language, composed at compile time into one binary per
product (ADR 0005). Pyoxide is the Python product.

## Language services (M3, ADR 0010, 0011)
First version done (#82-#87): `lsp-client` (framing over `process`,
cancellation, document sync, position encodings, pull and push
diagnostics, routing per feature), `code-intel-ui` (completion,
documentation, parameter info, navigation, intention and usages popups,
the Language Services widget; `ui::markdown`), `editor` (painting of
diagnostics, error stripe, inspection widget, next / previous problem,
hover input) and `py-lsp` (ruff, ty and basedpyright: lookup, settings,
server configuration, the one-time import of the reference's tool
settings, ruff rule names and Disable actions, the type engine widget).
Later: Find Usages tool window, rename, Problems tool window, inlay hints,
semantic tokens. Files changed outside the application reach the
services as watched-file changes (`file-watcher`, issue #36).
- Layering (`scripts/check-layers.py`): `code-intel`, `lsp-client` and
  `py-lsp` know no UI toolkit; only `lsp-client` sees protocol types;
  platform UI crates depend on `code-intel`, never on `lsp-client`; the
  editor does not depend on `code-intel`.

## Terminal, debugger, git and settings (proposed, ADR 0014-0018)
Planned crates; `terminal` exists (#156: sessions, snapshots, input
encoding, start rules, shell detection, the `ShellCustomizer` hook; the
integration scripts and the `terminal.*` commands come with their users).
Crates without a toolkit are tested without GPUI; UI
crates depend on the model crates, never on the protocol or backend
crates (`scripts/check-layers.py` gets these rules).

| crate | layer | GPUI | role | ADR |
|---|---|---|---|---|
| `terminal` | platform | no | sessions: PTY and emulator (`alacritty_terminal`, private to the crate), reader thread with the OSC 7 / 133 pre-scan, snapshots of the visible cells, input encoding, shell start rules and detection, shell-integration scripts, the `ShellCustomizer` hook, `terminal.*` engine commands | 0014 |
| `terminal-ui` | platform | yes | the grid element, the session view (selection, find, links), the Terminal tool window content | 0014 |
| `stream-framing` | platform | no | `Content-Length` framing, moved out of `lsp-client`, shared with `dap-client` | 0015 |
| `dap-client` | platform | no | the DAP client: adapter process, requests, events, reverse requests (`runInTerminal`); protocol types private | 0015 |
| `debugger` | platform | no | sessions, threads, frames, variables, breakpoints, watches, the `DebugAdapterProvider` hook, storage and import, `debug.*` engine commands | 0015 |
| `debugger-ui` | platform | yes | Debug tool window, Breakpoints dialog, breakpoint popup, inline values, value tooltips, editor wiring | 0015 |
| `console-ui` | platform | yes | the Run console view, moved out of `workbench` so that Run and Debug share it | 0015 |
| `vcs` | platform | no | the version control model and backend trait: file status, changes, the status map, line ranges against the base text (`imara-diff`), branches | 0016 |
| `git` | platform | no | the `git` executable: lookup, runner, parsers, `.git` readers, operations, repository detection, `git.*` engine commands | 0016 |
| `vcs-ui` | platform | yes | Commit tool window, changes tree, diff viewer tab, VCS widget and branch popup, git dialogs | 0016 |
| `settings` | platform | no | the application settings store (`settings.toml` and dedicated files), scopes, change events, `settings.get` / `settings.set` | 0017 |
| `settings-ui` | platform | yes | the Settings window, the page registry and page trait, pages that only edit the store | 0017, 0018 |
| `py-debug` | python | no | debugpy helper lookup, launch arguments, stoppable lines, inline value positions, Python debugger settings | 0015 |
| `pyoxide-ide` (extended) | python | yes | venv and uv activation in the terminal, the Python settings pages, the debug adapter provider | 0014, 0015, 0017 |

Dependencies: `terminal-ui` -> `terminal`, `console`, `ui`;
`debugger-ui` -> `debugger`, `console-ui`, `editor`, `ui`; `vcs-ui` ->
`vcs`, `editor`, `ui`; `git` -> `vcs`, `process`; `dap-client` ->
`stream-framing`, `process`; `lsp-client` -> `stream-framing`;
`workbench` -> the UI crates and `git`, and registers their tool windows,
settings pages and commands. Keymaps stay in `workbench` (ADR 0018).

## Cross-cutting foundations
Several of these areas need the same pieces of the shell. Each is built
once, before or with its first user:

- **Tool window slots.** `workbench::ToolWindow` knows Project and Run; the
  bottom area shows only the Run tool window and the left area only the
  project tool window; the Commit, Terminal and Version Control stripe
  buttons have no tool window behind them. Needed: tool windows
  registered with an area (left, bottom, right), a stripe group and
  order, an icon, a title, shortcut text and a view; one visible tool
  window per area (Project or Commit on the left; Run, Debug or Terminal
  at the bottom) with a size per area; activation, toggle and hide
  commands for each (`workbench.toggle-terminal`, `workbench.toggle-debug`,
  `workbench.toggle-commit`), stripe badges, and registration on first use
  as the Run tool window does now.
- **Balloon notifications.** `ui::notification` holds the balloon (kind
  information, warning or error, title, content, action links, close
  button, timed or sticky) and the stack (three shown, "N more" above,
  hover pauses the timer); `workbench::notifications` hosts it at the
  bottom right of the main window. `Workbench::notify` and the
  `workbench.notify` command show a balloon; `workbench.state` lists the
  open ones. Users: the run configuration import report now; commit, push,
  update and checkout results, debugger connection errors and terminal
  environment changes as those areas land. `Workbench::show_notice` stays
  for status bar texts. The Notifications tool window comes later.
- **Editor decorations.** The editor has diagnostic marks that move with
  edits, a link highlight and a problem cell after the end of a line, and
  keeps room right of the line numbers, but paints nothing there. Needed:
  range markers that move with edits (generalized from the diagnostic
  marks), gutter icons drawn over line numbers with hover, click,
  Alt-click, Shift-click and right-click events (breakpoints), a lane of
  change markers with hover and click (git), full-line backgrounds
  (execution point, breakpoint lines, diff), styled, clickable texts after
  the end of a line (inline values), and error-stripe marks in given
  colours. Owned by `editor`; it follows the soft-wrap work in the editor
  element (#119).
- **Editor tabs that are not editors.** The editor area's tabs hold one
  `Editor` each, keyed by path; read-only texts reuse that with a title as
  the path. Needed: a tab kind whose content is any view, with its own
  title, icon, tooltip and close handling: the diff viewer
  (`Commit: <file>`), later a terminal moved to the editor.
- **Non-modal dialog windows.** `ui::dialog_window` is modal only: the
  parent is blocked and its keystrokes dropped. Needed: a non-modal
  option (no blocker, the parent keeps its input, one instance per key
  brought to the front when opened again, focus handed back on close) for
  the Settings window.
- **Form widgets.** The widgets of the Run/Debug Configurations dialog
  (label column, text fields on one-line editors, combo boxes, fields with
  a browse button, checkbox rows, the bottom bar with its buttons) are
  private to `workbench::run::edit_dialog`. Needed: `ui::form` for layout
  and the widgets without text input (label column, rows and gaps, group
  headers, links, combo, checkbox rows, the bottom bar with the per-OS
  button order) and a one-line text field constructor in `editor` (the
  `ui` crate is below `editor`), used by the settings pages, the
  interpreter, git and debugger dialogs, and the Run/Debug dialog itself.
- **Application settings store.** Terminal, debugger and git options need
  the `settings` crate (ADR 0017) before their settings pages exist; their
  first versions read it with defaults.

The editor's context menu does not exist yet; the debugger's and git's
entries in it (Run to Cursor, Add to Watches, Git) follow when it is added.

## Planned layers
`core` (project model, settings, commands) -> `services` (LSP/DAP clients, VCS,
Docker, DB, terminal) -> `ui` (GPUI, ADR 0006) -> `app`. Decisions are recorded
in [adr/](adr/).

## External processes
Discovered and version-checked at runtime; the IDE degrades gracefully when
one is missing.

- The Python interpreter, ruff and a type checker (ADR 0010, 0011).
- Shells in the terminal, over a pseudo-terminal (ADR 0014): zsh, bash,
  fish, PowerShell, cmd.exe. Release builds for Windows ship
  `conpty.dll` and `OpenConsole.exe` from Windows Terminal.
- debugpy (ADR 0015): its adapter and launcher run with the project's
  interpreter from a helper folder shipped with the application (pinned
  version) and start the program being debugged.
- `git` (ADR 0016), from the settings, `PATH` or the usual install
  folders.
