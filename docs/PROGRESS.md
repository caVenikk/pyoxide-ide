# Progress

_Generated from `replica/features.csv` by `scripts/progress.py`; do not edit by hand._

**Overall parity: 43%** `█████████░░░░░░░░░░░`

| Area | Parity | Done | Partial | Todo |
|---|---|---|---|---|
| database | 71% `███████░░░` | 15 | 6 | 6 |
| db-gui | 0% `░░░░░░░░░░` | 0 | 0 | 8 |
| editor | 62% `██████░░░░` | 5 | 6 | 3 |
| settings-import | 55% `█████░░░░░` | 1 | 2 | 1 |
| python-intel | 67% `███████░░░` | 10 | 4 | 7 |
| run-debug | 20% `██░░░░░░░░` | 2 | 4 | 20 |
| terminal | 12% `█░░░░░░░░░` | 0 | 4 | 17 |
| vcs | 32% `███░░░░░░░` | 5 | 2 | 18 |
| settings | 53% `█████░░░░░` | 7 | 3 | 8 |
| docker | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| platform | 0% `░░░░░░░░░░` | 0 | 0 | 1 |
| packaging | 30% `███░░░░░░░` | 0 | 1 | 1 |
| python-env | 55% `██████░░░░` | 1 | 8 | 0 |

✅ done · 🟡 partial · ⬜ not started. Features marked (own) are additions beyond the reference and are not part of the parity score.

## database

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Oracle connection (thin driver) | must | db-oracle-thin over oracle-rs with a PL/SQL wrapper (works around oracle-rs bugs); no NNE, no cancel |
| ✅ | Oracle connection (Oracle Client / ODPI-C) | should | db-oracle-odpi: cancel, call timeouts, TNS/wallets/NNE/Kerberos; integration tests need Oracle Instant Client |
| ✅ | PostgreSQL connection | must | db-postgres (tokio-postgres, rustls) |
| ✅ | SQLite connection | must | db-sqlite (bundled SQLite) |
| 🟡 | Run SQL and view results | must | CLI and db.query command |
| ✅ | Typed result values | must | all four backends |
| ✅ | Schema introspection (tables / views / columns) | must | Oracle (ALL_* views, 19c-safe), PostgreSQL, SQLite |
| ✅ | Indexes constraints sequences procedures | should | Oracle, PostgreSQL, SQLite |
| ✅ | Connection profiles with keychain secrets | must | TOML profiles, OS credential store, env, prompt |
| ✅ | Password storage in KeePass (kdbx) | should | secrets: KDBX 4 store, master password in OS store / env / prompt |
| 🟡 | TLS and wallets | should | PostgreSQL all modes; Oracle Client tcps and wallets; thin: TCPS with ewallet.pem, no cwallet.sso; TCPS untested live |
| 🟡 | Query timeout and cancel | must | timeouts everywhere; cancel on SQLite, PostgreSQL, Oracle Client (not thin) |
| ✅ | Paginated results | must | incremental cursors on all backends |
| ⬜ | Export results (CSV / JSON) | should |  |
| ✅ | Connection diagnostics (own) | should | pyoxide-db diagnose |
| ✅ | Test connection (server and driver version) | must | pyoxide-db test / db.connection.test |
| ✅ | Bind parameters | should | all four backends |
| ✅ | Transaction mode (auto / manual) | should | all four backends |
| 🟡 | Read-only connections | should | SQLite, PostgreSQL; Oracle client-side best effort |
| ✅ | Oracle TNS alias and connect descriptors | should | Oracle Client (dsn as is); thin via oracle_net (Easy Connect, descriptors, tnsnames.ora) |
| ⬜ | Schema and object filters | should | Pro |
| ✅ | Object source and DDL | should | Oracle (ALL_SOURCE, DBMS_METADATA), PostgreSQL, SQLite |
| ⬜ | SSH tunnel | should | Pro; after MVP; must never block the UI (feedback) |
| ⬜ | Startup script / keep-alive / auto-disconnect | could | Pro |
| 🟡 | Large values: capped fetch of LOB / JSON / long text | must | max_value_bytes and Value::Truncated; lazy full fetch todo |
| ⬜ | Persistent schema cache across restarts | should | feedback: introspection slow or forgotten |
| ⬜ | Cancellable scoped introspection with progress | should | feedback |
| 🟡 | PostgreSQL Unix-socket connections | could | implemented, untested |
| ⬜ | External secret provider (command / password manager CLI) (own) | could | own feature; feedback |

## db-gui

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Database explorer tree | must | Pro |
| ⬜ | Data grid with paging | must | Pro; virtualized, renders visible cells only (feedback: grid freezes) |
| ⬜ | SQL console with history | should |  |
| ⬜ | Result table editing | could |  |
| ⬜ | Explain plan | could | Pro |
| ⬜ | Run a SQL file as a run configuration with results | should | feedback: top open request |
| ⬜ | Open a SQLite file directly | should | feedback |
| ⬜ | Date and number formats for grid, import and export | should | feedback |
| ⬜ | SQL*Plus commands in the Oracle console (DESCRIBE / EXEC) (own) | should | own feature; feedback |
| ⬜ | psql / sqlite3 meta-commands in the console (subset) (own) | could | own feature; feedback |

## editor

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Text buffer and editing | must | rope buffer, undo/redo, clipboard, IME, default keymaps and editing basics (pairing, smart indent, comment, line moves, extend selection, go to line); files changed on disk reloaded, File Cache Conflict, Reload All from Disk (#36) |
| 🟡 | Main window chrome (toolbar, tool-window stripes, status bar) | must | New UI layout and Islands themes; toolbar file path; run widget and Run tool window work for Current File; interpreter status widget; other stripe buttons and status widgets are placeholders; native caption on Windows/Linux |
| ✅ | Python syntax highlighting | must | tree-sitter off the UI thread mapped to theme text roles |
| 🟡 | Tabs and split views | must | tabs with modified marker and keyboard switching; no split views |
| 🟡 | File tree and project model | must | open folder, lazy tree, ignored-files list and excluded folders as in the reference (Mark Directory As > Excluded, .oxide store, one-time .idea import), context menu; New (File, Directory, Python File / Package), Rename, Delete to the trash, Cut / Copy / Paste, Copy / Move dialogs, drag and drop, Copy Path (#37); no multi-selection (#125), no Safe Delete or rename with usages (#123) |
| ✅ | Python packages and source roots in the tree | must | #120: package and pyproject-module icons, sources / test sources roots with hints, Mark Directory As (incl. the project root), .oxide [roots] with one-time .idea sourceFolder import; roots feed Go to File, run PYTHONPATH and module names (ADR 0013); Namespace Package and Not Excluded later |
| ✅ | File type icons by name pattern | must | #121: associations as data (exact name, name in any case, pattern, extension; later ones override), reference icons incl. Docker / Compose / .dockerignore / GitLab CI / GitHub / ignore files / INI / .env, our own YAML icon; unknown names typed from their first bytes off the UI thread; File Types settings page with #92 |
| ✅ | Editor soft wraps | must | #119: wrap over visual rows (caret, selection, Up/Down, Home/End, PageUp/Down, scroll, hit testing), incremental re-wrap with large texts off the UI thread, Soft-Wrap command (View > Active Editor), signs on the caret line or all, original indent, editor.toml masks (Editor > General page with #92), Python wrap rules; Home stop inside a continuation indent later |
| 🟡 | Search in file and project | must | find and replace in file (#31) and in files (#43: scopes, masks, editable preview, Replace in Files); missing: Find tool window (#65), multi-row selection (#66), undo across files (#67), comment/string filters (#50) |
| 🟡 | Navigation popups (Search Everywhere, Go to File / Action, Recent Files) | must | All / Files / Actions / Text tabs, ported name matcher, file:line, recent files; Classes and Symbols with M3; Preview toggle with a read-only preview shared with Find in Files (#118; look follow-ups #177); Find window (#65); the reference's ML ranking replaced by a fixed rule |
| ✅ | Navigation history (Back / Forward) | must | places and keys as in the reference, mouse buttons 4 / 5, view position restored (#103); only the primary caret is kept; Recent Locations and Last Edit Location later |
| ⬜ | Multi-cursor and column selection | should |  |
| ⬜ | Code folding | should |  |
| ⬜ | Live templates | should |  |

## settings-import

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Keymap import | must | reference keymap XML with its parent chain, Import Keymap… on the Keymap page (#40) |
| ⬜ | Code style import | should |  |
| 🟡 | Editor colour scheme import (.icls) | must | themes crate: schemes with parents and baseAttributes, report; no settings UI yet |
| 🟡 | UI theme import (.theme.json) | must | themes crate: theme.json and plugin jars (data files only), report; icon colour palette, run widget, tree, check box and button colours and sizes from the theme (#222, #225) |
| ✅ | Import data sources and passwords from PyCharm (own) | should | own feature; verified with the macOS keychain; KeePass and Windows DPAPI paths tested on synthetic data |

## python-intel

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Interpreter and virtualenv discovery | must | py-env: uv, PATH, venv, Poetry, conda, pyenv, py launcher; status bar widget and popup with the project's environments, the reference's interpreter name imported once; configured interpreters, the Python > Interpreter page and Add Python Interpreter: area settings (#75) |
| ✅ | Language services over LSP (start, stop, restart, status widget) | must | Community; ruff and ty / basedpyright over the platform LSP client (#83); Language Services widget and popup with restart / stop and server error output (#85) |
| 🟡 | External tools settings and discovery (ruff / ty / Pyright / Basedpyright / Pyrefly) | must | Community; lookup Sdk / Path / uvx as the reference, settings in .oxide, one-time .idea/pyLspTools.xml import (#87); Settings pages: area settings (#92); Pyright / Pyrefly not offered yet |
| ✅ | Type engine choice (Types widget) | could | Pro in the reference (Built-in / Pyrefly, observed); Pyoxide switches its type server here: ty / basedpyright (#87) |
| ✅ | Problems highlighted in the editor (severity, unused, deprecated, tooltip) | must | Community; from ruff and the type server (#84, #86: one hover popup with problems, first fix and documentation) |
| ✅ | Error stripe, inspection widget, next / previous problem (F2) | must | Community (#84) |
| ⬜ | Problems tool window | should | Community; needs generic tool windows |
| ✅ | Type checking | must | Community in the reference (PSI inspections or an LSP tool); Pyoxide: ty (ADR 0011), basedpyright as an option; type problems shown as warnings as the built-in engine does |
| ✅ | Code completion (basic, auto-popup, auto-import) | must | Community (#85, #87); completion characters, documentation in the lookup and postfix templates later |
| ✅ | Quick documentation (F1 / hover) | must | Community (#86) |
| ✅ | Parameter info | must | Community (#86) |
| 🟡 | Go to declaration (Cmd+B / Cmd+click) and type declaration | must | Community; Cmd+B, Cmd+click, chooser, usages on the declaration (#86), Back / Forward (#103); Go to Type Declaration missing |
| 🟡 | Find usages and Show Usages popup | must | Community; Show Usages popup (#86); Find Usages tool window later |
| ⬜ | Highlight usages of the symbol at the caret | should | Community |
| ✅ | Quick fixes and intention actions (Alt+Enter) | must | Community; ruff fixes with Disable for file / line / project (#86, #87); intention preview later |
| ✅ | Reformat code and optimize imports with ruff | must | Community (#86, #87) |
| ⬜ | Rename (Shift+F6) | should | Community |
| ⬜ | Other refactorings (extract, inline, move, change signature) | could | Community |
| ⬜ | Go to Class / Go to Symbol and File Structure | should | Community; Search Everywhere Classes and Symbols tabs |
| ⬜ | Inlay hints | could | Community |
| ⬜ | Semantic highlighting | could | Community |

## run-debug

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Run the current file (Run tool window: console with ANSI colours and links, stdin, stop / rerun) | must | #45; uv run for uv environments; soft wraps, print and console find later |
| 🟡 | Run configurations | must | Python and Shell Script types, one-time import from .idea (personal and shared, unsupported types kept read-only), run widget dropdown, Edit Configurations dialog, temporary configurations (#74, ADR 0012); no debug (#90), no gutter or Project view Run entries, interpreter as text until #75 |
| ⬜ | Debug the current file and run configurations (debugpy over DAP) | must | Community (pydevd; debugpy through a separate plugin in the reference); debugpy 1.8.21 shipped with the app, ADR 0015 (#90) |
| ⬜ | Run widget and configurations during a debug session (Pause / Resume / Rerun, Debug in the dropdown and dialog) | must | Community; Debug buttons of #74 enabled (#90) |
| ⬜ | Debug tool window (session tabs, toolbar, Threads & Variables, Console with stdin) | must | Community; Console through DAP runInTerminal like the Run console (#90) |
| ⬜ | Stepping and session control (step over / into / into my code / out, run to cursor, resume, pause, stop, rerun) | must | Community; keys of the macOS and Windows keymaps (#90) |
| ✅ | Line breakpoints in the gutter (click, Cmd+F8 / Ctrl+F8, temporary, Alt+click, icons over line numbers) | must | Community; stoppable lines as the reference, hover icon, state icons, follow edits (#164) |
| 🟡 | Breakpoint conditions, properties popup, Breakpoints dialog and Mute Breakpoints | must | Community; condition, popup, dialog (#164); no completion in the condition, Mute shown per project |
| 🟡 | Python exception breakpoints (Any exception on termination / on raise) | must | Community; Any exception + built-in classes in the dialog (#164); one class needs exceptionOptions (#200) |
| ⬜ | Frames list and thread chooser (library frames folded) | must | Community (#90) |
| ⬜ | Variables tree (lazy and paged children, Special Variables, Set Value, Copy Value) | must | Community (#90) |
| ⬜ | Execution point and inline values in the editor | must | Community (#90) |
| 🟡 | Breakpoints saved per project with one-time import from .idea | must | Community; saved in .oxide/workspace.toml (#164); .idea/workspace.xml import to check |
| ⬜ | Evaluate Expression dialog, Quick Evaluate and value tooltip | should | Community |
| ⬜ | Watches and the evaluate field in Variables | should | Community; watches kept per run configuration |
| ⬜ | Logging breakpoints (log message, stack trace, Evaluate and log) | should | Community |
| ⬜ | Smart Step Into | should | Community; on by default in the reference |
| ⬜ | Debug Console (prompt in the current frame) | should | Community |
| ⬜ | Attach to Process | should | Community |
| ⬜ | Subprocess debugging | should | Community; debugpy child sessions over TCP |
| ⬜ | Debugger settings pages (Python Debugger, Debugger, Stepping, Data Views) | should | Community |
| ⬜ | Jump To Cursor and the Run to Cursor inlay | could | Community |
| ⬜ | Dependent breakpoints (disable until another is hit) | could | Community |
| ⬜ | Remote, Docker and WSL debugging | could | remote interpreters |
| ⬜ | Data views for numpy and pandas and type renderers | could | Community |
| ⬜ | Test runner (pytest) | should |  |

## terminal

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Terminal tool window (Alt+F12, stripe button, Hide) | must | Community (Reworked engine, the 2026.2 default); ADR 0014 (#89) |
| 🟡 | Local shell sessions over a pseudo-terminal (PTY on macOS and Linux, ConPTY on Windows) | must | Community; default shell per OS, login and interactive options, environment (#89) |
| 🟡 | VT/xterm emulation (16 / 256 / 24-bit colours, alternate screen, resize, mouse reporting, bracketed paste, scrollback) | must | Community; candidate alacritty_terminal (#89) |
| ⬜ | Terminal colours and font from the theme (BLOCK_TERMINAL keys, minimum contrast) | must | Community (#89) |
| ⬜ | Terminal tabs (new, close with confirmation, rename, unique names, close on exit) | must | Community (#89) |
| ⬜ | Select, copy, paste, select all and clear in the terminal | must | Community (#89) |
| 🟡 | Shell integration (prompt and command marks, working directory) for zsh / bash / fish / PowerShell | must | Community; OSC 7 / OSC 133 scripts for zsh, bash, sh, fish, PowerShell (#159); used once the Terminal tool window starts sessions through workbench::terminal::plan |
| 🟡 | Virtual environment activation and start directory in the terminal | must | Community; project venv / uv activated through its activate script or env merge, [python.terminal] activate-virtualenv imported from .idea (#159); start directory and conda later |
| ⬜ | Links in terminal output (URLs, file paths with line:column, Python tracebacks) | must | Community (#89) |
| ⬜ | Find in terminal output | must | Community (#89) |
| ⬜ | Terminal key routing (Override IDE shortcuts, allowed IDE actions, Option as Meta) | must | Community (#89) |
| ⬜ | Terminal tabs restored on project open | should | Community |
| ⬜ | New Predefined Session popup (detected shells) | should | Community |
| ⬜ | Open In > Terminal | should | Community |
| ⬜ | Command separators in the terminal | should | Community; needs shell integration |
| ⬜ | Terminal tab and output context menus | should | Community |
| ⬜ | Settings > Tools > Terminal page with one-time import from .idea | should | Community; after the settings window (#92) |
| ⬜ | Split terminal tabs and Move to Editor | could | Community |
| ⬜ | Shell command completion in the terminal | could | Community |
| ⬜ | Paste from History and command history search in the terminal | could | Community |
| ⬜ | Conda environment activation in the terminal | could | Community |

## vcs

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Git repository detection and git executable lookup | must | Community; the git executable for every operation, ADR 0016 (#91) |
| ✅ | File status colours in the project tree, editor tabs and file lists (ignored included) | must | Community; tree, tabs, Recent Files and Go to File rows, unsaved edits at once (#169) |
| 🟡 | Gutter change markers, error stripe marks and marker popup (Rollback Lines, Next / Previous Change) | must | Community; markers within 5 ms of a keystroke, popup with Rollback / Copy / Next / Previous (#169); no Show Diff (no diff viewer yet), no Commit this change |
| ✅ | Commit tool window (changes tree with check boxes, message, Commit, legend, errors) | must | Community; Cmd+K / Ctrl+K, non-modal commit, Amend, options, commit checks (#170); no grouping by directory, Shelve, change lists |
| ⬜ | Amend and commit options (author, sign-off, hooks) | must | Community (#91) |
| ✅ | Rollback of files (Rollback Changes dialog) | must | Community (#170) |
| ⬜ | Side-by-side diff viewer (Commit: <file> tab, word highlights, F7 navigation, editable right side) | must | Community (#91) |
| ✅ | VCS widget in the main toolbar (branch, detached HEAD, operation state, ahead / behind) | must | Community; branch / tag / commit, merging / rebasing state, incoming and outgoing arrows, tooltips (#28) |
| 🟡 | Branches popup (Recent / Local / Remote, search, checkout, new, rename, delete, Smart Checkout) | must | Community; groups, search, branch submenu, New / Rename / Checkout dialogs, Smart Checkout by stash (#28); Update / Push / Fetch wait for #170 #172, favourites not saved |
| ⬜ | Push dialog and Commit and Push (upstream for new branches, force with lease, Push Rejected) | must | Community; credentials through the user's helper and ssh-agent (#91) |
| ⬜ | Update Project, Fetch and Pull | must | Community (#91) |
| ⬜ | Create Git Repository for a project without version control | must | Community (#91) |
| ⬜ | Git main menu and VCS Operations popup | must | Community (#91) |
| ⬜ | Annotate with Git Blame | should | Community |
| ⬜ | Git log tool window and file history | should | Community |
| ⬜ | Merge conflict resolution (three-way merge) | should | Community |
| ⬜ | Stash and shelve UI | should | Community |
| ⬜ | Credentials prompt for HTTPS and SSH | should | Community; askpass helper |
| ⬜ | Unified diff viewer | could | Community |
| ⬜ | Changelists and partial commits (chunks, lines) | could | Community |
| ⬜ | Cherry-pick and interactive rebase | could | Community |
| ⬜ | Commit checks (reformat, optimize imports, ruff) | could | Community |
| ⬜ | .gitignore support and Add to .gitignore | could | Community |
| ⬜ | Compare with branch, worktrees and tags UI | could | Community |
| ⬜ | Clone and share on a hosting service | could | Community |

## settings

| | Feature | Priority | Notes |
|---|---|---|---|
| ✅ | Settings window (page tree with search, Revert changes, Apply / OK / Cancel, non-modal, Cmd+, / Ctrl+Alt+S) | must | Community; ADR 0017 (#92) |
| 🟡 | Application settings store with one-time pick-up of the reference's settings (theme, scheme, keymap, font, soft wraps, autosave) | must | own store (settings.toml); reads the reference's XML data files once (#92, #40) |
| ✅ | Appearance page: theme choice with live switch, Custom themes and Import Theme… | must | Community; Theme combo (grouped, live, Cancel keeps it), editor scheme combo, Import Theme… (#153); Sync with OS, Accessibility and UI Options groups not drawn yet |
| 🟡 | Theme quick switch with preview (gear > Theme, Quick Switch Scheme) | must | Community; gear Theme… popup with 500 ms preview, Esc restores; Quick Switch Scheme (Ctrl+`) has only the Theme row (#153) |
| ✅ | System Settings page (autosave and sync of external changes) | must | Community; defaults in code today (#92) |
| ✅ | Keymap choice and runtime switch (macOS, Windows, VS Code; gear > Keymap) | must | Community; ADR 0018; VS Code keymaps from Community data, Settings, gear menu, Quick Switch Scheme (#40); no Zed keymap (no permissive source) |
| ✅ | Keymap page with a read-only actions tree and search by name and by shortcut | must | Community (#40) |
| ✅ | Editor > Font page | must | Community; font, size, line height, ligatures, Restore defaults, fallback font, preview; editors and Run console follow on Apply (#154); no weights / character variants |
| 🟡 | Editor > General page (soft wraps) | must | Community; Soft Wraps group only (#154) |
| ⬜ | Python > Interpreter page (combo, Show All, add / remove / rename) | must | Community (#75) |
| ⬜ | Add Python Interpreter dialog (new virtualenv or uv, existing interpreter) | must | Community; through py-env (#75) |
| ⬜ | Configured interpreters as an application list with one-time import from the reference | must | Community; interpreters of the reference that still exist (#75) |
| ✅ | Python > Type Engine and Python > Tools > External Tools pages | must | Community; project pages, off the UI thread (#154); no Configure dialog or Install / Upgrade yet |
| ⬜ | Search highlight on settings pages (spotlight) | should | Community |
| ⬜ | Keymap editing (custom keymaps, edit shortcuts) | should | Community |
| ⬜ | Editor > File Types page | should | Community; associations from #121 |
| ⬜ | First-run window (import, choose theme, choose keymap) | should | Community (#40) |
| ⬜ | Appearance options beyond the theme (zoom, UI font, UI options) | could | Community |

## docker

| | Feature | Priority | Notes |
|---|---|---|---|
| ⬜ | Docker containers and images | should |  |

## platform

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Command registry and palette (own) | must | registry and JSON channel; no palette yet |
| ⬜ | Plugin API (WASM) | could |  |

## packaging

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | Windows / macOS / Linux builds | must | CI builds all three |
| ⬜ | Signed installers | should |  |

## python-env

| | Feature | Priority | Notes |
|---|---|---|---|
| 🟡 | uv: create virtualenv from the IDE | must | engine and pyoxide CLI (env venv); no UI yet |
| 🟡 | uv: project init and sync (pyproject.toml / uv.lock) | must | engine and CLI (env init/sync/lock); no UI yet |
| 🟡 | uv: install and manage Python versions | should | engine and CLI (env python-install); no UI yet |
| 🟡 | Package manager panel (list / install / upgrade / remove) | must | engine and CLI (env packages/add/remove, outdated); panel UI todo |
| 🟡 | Package search and version picker | should | PyPI lookup (env search); picker UI todo |
| 🟡 | Dependency groups and extras in pyproject.toml | should | read and edit via uv add --group/--optional; no UI yet |
| ✅ | uv run as run configuration | should | the current file and Python configurations run through uv run in a uv environment, standalone uv run configurations imported as Python ones (#45, #74) |
| 🟡 | PEP 723 inline script dependencies | could | parsed by the project model |
| 🟡 | Other managers (poetry / conda / pipenv / pip) | should | discovery of Poetry/conda/Pipenv, pip fallback for packages |
