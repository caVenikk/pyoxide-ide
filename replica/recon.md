# Recon map: PyCharm (desktop)

One section per feature area. Sources are public documentation and observed
behaviour only (see `docs/clean-room.md`). Reference screenshots stay in the
git-ignored `replica/screens/`.

---

## Area: db (database client engine)

Scope: the engine behind the database tools: connections, statement
execution, result paging, cancel and timeouts, typed values, schema metadata,
connection settings and secrets. The GUI on top (explorer tree, console, data
grid) is area `db-gui`.
Date: 2026-10-09

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Data source settings reference | https://www.jetbrains.com/help/pycharm/data-sources-and-drivers-dialog.html | General, Options, SSH/SSL, Schemas, Advanced tabs |
| 2 | Configure connection | https://www.jetbrains.com/help/pycharm/configuring-database-connections.html | test connection, save password modes |
| 3 | Run queries | https://www.jetbrains.com/help/pycharm/run-a-query.html | execute, cancel running statements |
| 4 | Rows / paging | https://www.jetbrains.com/help/pycharm/rows.html | page size setting, default 500 |
| 5 | Sessions | https://www.jetbrains.com/help/pycharm/managing-connection-sessions.html | transaction mode, cancel per session |
| 6 | Query results | https://www.jetbrains.com/help/pycharm/viewing-query-results.html | result tabs, in-editor results |
| 7 | Oracle Database Reference (data dictionary) | https://docs.oracle.com/en/database/oracle/oracle-database/19/refrn/ | ALL_* views for introspection |
| 8 | Oracle Database Error Messages | https://docs.oracle.com/en/database/oracle/oracle-database/19/errmg/ | ORA-/TNS- codes for diagnostics |
| 9 | PostgreSQL system catalogs | https://www.postgresql.org/docs/current/catalogs.html | pg_catalog, information_schema |
| 10 | SQLite schema table and PRAGMAs | https://www.sqlite.org/schematab.html, https://www.sqlite.org/pragma.html | sqlite_schema, table_info, index_list |

### Core loop
Configure a data source once, test it, then run SQL and browse results and
schema objects without leaving the IDE.

### Screens (engine-facing; the GUI is area db-gui)

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-DB01 | Data source settings | Database tool window, `+`, data source | connection parameters | host, port, database/service, user, password + save mode, URL, driver, Test Connection | empty, filled, testing, success (server + driver version), error |
| S-DB02 | Options tab | S-DB01 | session behaviour | read-only, transaction control Auto/Manual, keep-alive, auto-disconnect, startup script, session control | default, changed |
| S-DB03 | SSH/SSL tab | S-DB01 | network security | SSH tunnel, SSL: CA file, client cert/key, truststore | off, on |
| S-DB04 | Schemas tab | S-DB01 | what to introspect | schema pattern, object filter | all, filtered |
| S-DB05 | Query console | data source, New, Query Console | run SQL | editor, execute, cancel, results | idle, running, result, error, cancelled |
| S-DB06 | Result grid | S-DB05 or open a table | view rows | page size (default 500), paging, NULL display, typed cells | empty, page, last page, error |

### Flows

```
F-DB01 Connect to a database
    S-DB01 -> Test Connection -> success shows server and driver versions
    edge: wrong password, unknown service/database, host unreachable,
          TLS required, missing client library (Oracle thick), timeout

F-DB02 Run a query and page through results
    S-DB05 -> execute -> S-DB06 first page -> next pages
    edge: zero rows, millions of rows, very wide values (LOBs), NULLs,
          exact decimals, time zones, binary data, a DML statement

F-DB03 Cancel a long statement
    S-DB05 -> execute slow statement -> Cancel Running Statements
    edge: cancel during fetch, cancel when nothing runs, server ignores cancel

F-DB04 Browse schema metadata
    explorer -> schema -> tables/views/sequences/routines -> columns, keys,
    indexes, constraints, source
    edge: no privileges on dictionary views, thousands of objects,
          quoted mixed-case names

F-DB05 Manage connections and passwords
    S-DB01 -> save password (never / session / forever) -> reconnect later
```

### Inferred data model

```
DataSource   name, driver/backend, host, port, database|service|sid|url, user,
             password save mode, ssl {ca, cert, key, mode}, ssh, read_only,
             tx_mode, keep_alive, auto_disconnect, startup_script,
             schema filter, connection properties
             evidence: sources 1, 2, 5    confidence: high
Session      data source, tx mode, running statements
             evidence: source 5           confidence: high
ResultPage   columns (name, type), rows, page size, has_more
             evidence: sources 4, 6        confidence: high
```

Pyoxide mapping: `DataSource` = connection profile (TOML, ADR 0004);
`Session` = `db_core::Connection`; `ResultPage` = `Cursor::next_page`.

### Feature matrix
See `replica/features/`, areas `database` and `db-gui`. Database tools
are a PyCharm Professional feature; they are promoted to MVP priorities here.

### Out of scope for the MVP
- JDBC drivers and "any driver" support; engines beyond Oracle, PostgreSQL,
  SQLite.
- SSH tunnels (planned after the MVP).
- Kerberos/LDAP for the thin Oracle driver (available through the Oracle
  Client backend).

### Size
Engine: 4 drivers, 1 contract, profiles, diagnostics. Hard parts: typed
decoding per backend, cancel semantics per protocol, Oracle environments that
the thin protocol cannot reach. Size: L (about 30 to 40 agent-hours).

---

## Area: editor (application shell and code editor)

Scope: the main window of the reference IDE in its current default look
(New UI with "Islands": rounded panels on a frame background), dark and
light, and the plain-text side of the code editor: open a folder as a
project, browse it, open files in tabs, edit, undo, select, copy, save,
syntax highlighting. Code intelligence (completion, inspections, navigation
by symbol) is area python-intel; run, debug and version control have their
own areas.
Date: 2026-10-11

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | User interface overview | https://www.jetbrains.com/help/pycharm/guided-tour-around-the-user-interface.html | main window parts, status bar widgets |
| 2 | New UI | https://www.jetbrains.com/help/pycharm/new-ui.html | main toolbar, tool window stripes, compact mode |
| 3 | Tool windows | https://www.jetbrains.com/help/pycharm/tool-windows.html | stripes, show and hide, view modes, shortcuts |
| 4 | Project tool window | https://www.jetbrains.com/help/pycharm/project-tool-window.html | tree, views, header actions |
| 5 | Editor basics | https://www.jetbrains.com/help/pycharm/using-code-editor.html | caret, selection, scrolling, gutter |
| 6 | Editor tabs | https://www.jetbrains.com/help/pycharm/managing-editor-tabs.html | open, close, pin, modified marker, tab limit |
| 7 | Saving and reverting | https://www.jetbrains.com/help/pycharm/saving-and-reverting-changes.html | autosave triggers, Save All |
| 8 | Undo and redo | https://www.jetbrains.com/help/pycharm/undoing-and-redoing-changes.html | per-file undo, grouping |
| 9 | Multiple carets and selection | https://www.jetbrains.com/help/pycharm/multicursor.html | caret cloning, column selection |
| 10 | Keyboard shortcuts | https://www.jetbrains.com/help/pycharm/mastering-keyboard-shortcuts.html | default keymaps per OS |
| 11 | Opening projects | https://www.jetbrains.com/help/pycharm/opening-reopening-and-closing-projects.html | open a folder, recent projects |
| 12 | Large files | https://www.jetbrains.com/help/pycharm/file-size-limit.html | size limits, read-only preview of huge files |
| 13 | IntelliJ Platform Community, Apache-2.0 | https://github.com/JetBrains/intellij-community | New UI theme files and metrics (`platform/platform-resources/src/themes/`, `JBUI.java`), default keymaps (`platform/platform-resources/src/keymaps/`), New UI icons (`platform/icons/src/expui/`) |

No reference screenshots exist locally yet. Metrics come from source 13;
the comparisons that still need screenshots are listed in `replica/perf.md`
(fidelity section).

### Core loop
Open a folder, find a file in the project tree, edit it with highlighting,
undo freely, save, move on to the next file in another tab.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-ED01 | Main window | start the IDE with a project | frame for everything | main toolbar in the title bar area, left and right tool window stripes, tool window islands, editor island, status bar | no project, project open, tool window hidden |
| S-ED02 | Main toolbar | always visible at the top | project and branch switchers, run controls, search, settings | project widget (coloured badge with initials, name, chevron), version control widget (branch icon, name, chevron), run configuration widget with run and debug buttons, search and settings icon buttons, window buttons on Windows and Linux | normal, hover, pressed, no project |
| S-ED03 | Tool window stripes | left and right edges | open and close tool windows | square icon buttons, top and bottom groups | idle, hover, selected (window open) |
| S-ED04 | Project tool window | stripe button, Cmd+1 / Alt+1 | browse files | header with title and view chooser, header actions (locate file, expand, collapse, options, hide), virtualized tree with chevrons, file type icons, root row with the path in muted text | empty, loading, filled, focused, selection without focus |
| S-ED05 | Editor tabs | open a file | switch between open files | one tab per file: icon, name, close button on hover and on the selected tab; underline under the selected tab; overflow when tabs do not fit | none open, one, many, modified, hover, selected |
| S-ED06 | Editor | a tab | edit text | gutter with line numbers (the caret line number brighter), text area with caret, selection, current line highlight, indent guides, right margin line, scrollbars that appear on scroll or hover, inspection widget at the top right | empty file, small, 100k lines, read-only, huge (preview only) |
| S-ED07 | Empty editor area | no tab open | hints | centred list of actions with their shortcuts (search everywhere, go to file, recent files, navigation bar), a hint that files can be dropped here | shown after the last tab closes |
| S-ED08 | Status bar | bottom edge, on the frame background | file and caret facts | path of the current file on the left; widgets on the right: caret position (line:column, selected characters), line separator, encoding, indentation, interpreter, read-only lock | no file, file, selection, background task |
| S-ED09 | Open folder dialog | File > Open, project widget | pick a folder | native folder chooser | cancelled, chosen |

### Flows

```
F-ED01 Open a folder as a project
    S-ED09 -> folder chosen -> S-ED01 with the folder name in the project
    widget and the tree in S-ED04 showing the root expanded one level
    edge: tens of thousands of files, hidden and ignored folders (.git,
          virtual environments, caches), symbolic link loops, unreadable
          folders

F-ED02 Open a file
    S-ED04 double click or Enter on a file -> new tab selected in S-ED05,
    caret at the start, focus in the editor; a file already open just
    selects its tab
    edge: binary file, invalid UTF-8, CRLF line separators, BOM, 10 MB
          file, file removed on disk after opening

F-ED03 Edit and save
    type -> text changes, caret moves, the status bar caret position
    follows; files are written by Save All (Cmd+S / Ctrl+S) and
    automatically when the window loses focus or a tab closes; a modified
    tab can show a marker
    edge: save fails (read-only file, disk full); line separators and BOM
          are preserved; the trailing newline stays as typed

F-ED04 Undo and redo
    Cmd+Z / Ctrl+Z undoes the last group of edits (consecutive typing is
    one group), Cmd+Shift+Z / Ctrl+Shift+Z redoes; caret and selection
    return to where they were
    edge: undo after save makes the file modified again; nothing to undo
          right after opening; a new edit clears redo

F-ED05 Move the caret and select
    arrows by character and line, Alt/Ctrl+arrows by word, Cmd+arrows or
    Home/End by line, Cmd+Up/Down or Ctrl+Home/End by document, Page
    Up/Down by screen; Shift extends the selection; a click places the
    caret, drag selects, double click selects a word, triple click a line,
    Shift+click extends; Cmd+A / Ctrl+A selects all
    edge: vertical moves keep the preferred column across short lines;
          tabs and wide characters; selections over 100k lines

F-ED06 Clipboard
    copy and cut the selection; without a selection copy and cut take the
    whole caret line; paste replaces the selection
    edge: CRLF text pasted into an LF file is normalised; a whole copied
          line is pasted above the caret line

F-ED07 Close tabs
    close button or Cmd+W / Ctrl+F4 closes the selected tab; the
    neighbouring tab becomes selected; closing the last tab shows S-ED07
    edge: a modified file is saved on close, without a prompt

F-ED08 Large files
    a 100k-line file opens at once, scrolls at display rate and types
    without delay; files above the editor size limit open as a read-only
    preview
    edge: very long lines, a single line of several megabytes

F-ED09 Tool windows
    stripe button or Cmd+1 / Alt+1 toggles the project tool window;
    Escape in a tool window returns focus to the editor; Shift+Escape
    hides it
```

### Components

| component | variants | states | used on |
|---|---|---|---|
| Island panel | tool window, editor | focused, unfocused | S-ED01 |
| Icon button | toolbar, stripe, header action | idle, hover, pressed, selected, disabled | S-ED02, S-ED03, S-ED04 |
| Toolbar widget | project, branch, run configuration | idle, hover, open | S-ED02 |
| Tree row | directory (chevron), file | idle, hover, selected with focus, selected without focus | S-ED04 |
| Tab | editor tab | idle, hover, selected, modified | S-ED05 |
| Scrollbar | vertical, horizontal | hidden, visible on scroll or hover, dragged | S-ED04, S-ED06 |
| Status widget | text, icon and text | idle, hover | S-ED08 |
| Tooltip | text, text with shortcut | shown after a delay | everywhere |

### Inferred data model

```
Project      root folder, name, excluded folders, open files (order,
             selected), tool window layout
             evidence: sources 4, 11                    confidence: high
Document     path, text, line separator, encoding, BOM, modified flag,
             undo history, read-only flag, modification stamp on disk
             evidence: sources 5, 7, 8                  confidence: high
EditorState  carets with selections, preferred column, scroll position
             evidence: sources 5, 9                     confidence: high
Keymap       action -> shortcuts per context, parent keymap
             evidence: sources 10, 13                   confidence: high
```

Pyoxide mapping: `Project` is the crate `project`, `Document` the crate
`text` (rope buffer and history), `EditorState` the crate `editor`,
`Keymap` the TOML keymaps of the crate `workbench` (ADR 0007).

### Feature matrix
See `replica/features/`, area `editor`: text buffer and editing, Python
syntax highlighting, tabs, file tree and project model are this slice;
search, multiple carets, folding and live templates come later.

### Out of scope for this slice
- Split editors, detached windows, tool window docking modes, drag and drop
  of tabs.
- Code intelligence, run, debug and version control (placeholders in the
  toolbar and stripes).
- Settings dialog, welcome screen with recent projects, menus other than
  the native macOS menu bar.

### Size
Screens 9, flows 9, entities 4. Hard parts: an editor within the latency
budget on 100k-line files, IME, faithful metrics without reference
screenshots, headless tests through commands. Size: XL (about 40 to 60
agent-hours).

---

## Area: python-intel (code intelligence for Python)

Scope: what the reference shows while Python code is edited: problems
highlighted in the editor with quick fixes, code completion, quick
documentation, parameter info, navigation to declarations and usages,
reformatting and import optimisation, plus the external language services
that feed some of it (ruff, ty, Pyright, Basedpyright, Pyrefly), their status
widget and settings. Interpreter discovery is covered by #15 / #45; running
and debugging is area run-debug.
Date: 2026-10-13. Reference: PyCharm 2026.2.3 (IntelliJ Community tag
`pycharm/2026.2.3`).

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | LSP tools in PyCharm | https://www.jetbrains.com/help/pycharm/lsp-tools.html | External Tools page, ruff, ty, Pyright, Basedpyright, Pyrefly, Black |
| 2 | Python type engine | https://www.jetbrains.com/help/pycharm/python-type-engine.html | Built-in vs Pyrefly, what an external engine supplies |
| 3 | Code completion | https://www.jetbrains.com/help/pycharm/auto-completing-code.html | basic completion, auto-popup, lookup keys |
| 4 | Quick documentation, parameter info | https://www.jetbrains.com/help/pycharm/viewing-reference-information.html | F1 / Ctrl+Q, hover, Cmd+P / Ctrl+P |
| 5 | Navigation | https://www.jetbrains.com/help/pycharm/navigating-through-the-source-code.html | Go to Declaration or Usages, type declaration |
| 6 | Find usages | https://www.jetbrains.com/help/pycharm/find-highlight-usages.html | Find Usages tool window, Show Usages popup, highlight usages |
| 7 | Intention actions and quick fixes | https://www.jetbrains.com/help/pycharm/intention-actions.html | Alt+Enter, submenus |
| 8 | Code inspections, Problems tool window | https://www.jetbrains.com/help/pycharm/code-inspection.html | severities, inspection widget, F2 |
| 9 | Reformat and optimize imports | https://www.jetbrains.com/help/pycharm/reformat-and-rearrange-code.html | Cmd+Alt+L / Ctrl+Alt+L, Ctrl+Alt+O |
| 10 | IntelliJ Platform LSP API | https://plugins.jetbrains.com/docs/intellij/language-server-protocol.html | client features, Language Services widget; free in unified PyCharm since 2025.1 |
| 11 | IntelliJ Community at `pycharm/2026.2.3`, Apache-2.0 | https://github.com/JetBrains/intellij-community/tree/pycharm/2026.2.3 | `platform/lsp`, `platform/lsp-impl` (client, document sync, highlighting, completion, hover, formatting, widget items), `platform/lang-impl/.../lsWidget` (Language Services widget), `python/python-lsp-core`, `python-lsp-impl`, `python-pytools(-ui)`, `python-ruff`, `python-ty`, `python-pyright`, `python-pyrefly`, `python-type-engine`, `python-psi-impl/.../codeInsight/completion` (Python completion details), keymaps in `platform/platform-resources/src/keymaps/` |
| 12 | LSP 3.17 specification | https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/ | protocol |
| 13 | ruff editor features and settings | https://docs.astral.sh/ruff/editors/features/, https://docs.astral.sh/ruff/editors/settings/ | `ruff server`: diagnostics, code actions, formatting |
| 14 | ty language server and editor settings | https://docs.astral.sh/ty/features/language-server/, https://docs.astral.sh/ty/reference/editor-settings/ | `ty server` |
| 15 | basedpyright, Pyright, Pyrefly docs | https://docs.basedpyright.com/, https://github.com/microsoft/pyright, https://pyrefly.org/en/docs/IDE-features/ | alternative servers |

Reference screenshots of the language services widget exist locally
(git-ignored); the remaining visual details are listed as open questions in
the issues. Server measurements (latency, memory, feature coverage) on a
private mid-size project are summarised in ADR 0011.

### How the reference is built (matters for the clone)
- Completion, navigation, find usages, rename, quick documentation and
  parameter info come from PyCharm's own Python engine (PSI), not from a
  language server. LSP tools are **off by default**; when enabled, their
  diagnostics and quick fixes are added to the editor, and some of their
  completion items, hover targets and declarations are merged with the
  engine's results (source 11, `PyLspToolIntegrationProvider`, per-tool
  `*Configuration` defaults).
- Feature defaults per tool (Configure dialog): Ruff: inspections,
  formatting, import optimizer. Pyright: inspections, completions.
  Basedpyright: inspections, completions, inlay hints. ty: inspections,
  completions, inlay hints, documentation. Pyrefly: inspections.
- Rename, highlight usages and semantic highlighting never use a language
  server for Python.
- The type engine (Built-in by default, Pyrefly offered for single-module
  projects, ty behind a registry flag) only replaces type inference under
  the PSI features.
- Pyoxide has no PSI engine. The clone keeps the reference's editor UX
  (lookup, popups, keys, highlighting) and takes the data from a type
  server running by default in place of the built-in engine, plus ruff for
  lint, quick fixes, formatting and import sorting.

### Core loop
Open a Python file, see problems highlighted as you type, complete code
with the popup, read documentation and parameter info, jump to a
declaration and back, fix a problem with Alt+Enter, reformat.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-PI01 | Editor highlighting of problems | open a file with problems | show diagnostics in place | wavy underline per severity (error red, warning orange, weak warning), greyed unused code (whole unused import statement), struck deprecated name at the call site; one hover popup with every problem at the mouse (message lines, grey source and rule link `Ruff(unused-import)` that opens the rule page in the browser), an actions row (first quick fix with Alt+Shift+Enter, More actions... with Alt+Enter) and the documentation of the symbol | none, errors, warnings, stale while typing |
| S-PI02 | Error stripe and inspection widget | right edge and top-right of the editor | file overview | stripe marks per problem (min 2 px, colour per severity), widget with an icon and count per severity present (counted per problem), up / down arrows, click opens Problems | clean (check mark), problems, analysing |
| S-PI03 | Completion popup (lookup) | type an identifier character or `.`, Ctrl+Space | complete code | rows: kind icon, name with matched chars highlighted, grey tail text, right-aligned grey type text; keywords bold, deprecated struck; first row selected | loading, items, no suggestions, incomplete (re-query) |
| S-PI04 | Quick documentation popup | F1 / Ctrl+J (macOS), Ctrl+Q; mouse hover after 500 ms | read docs | rendered Markdown/HTML, code blocks highlighted, links, toolbar | loading, content, nothing found |
| S-PI05 | Parameter info popup | Cmd+P / Ctrl+P, auto after `(` (1000 ms) | show signatures | above the call, one line per overload with parameter names only, current parameter emphasised | one, several overloads, no parameters |
| S-PI06 | Declaration navigation | Cmd+B / Ctrl+B, Cmd/Ctrl+click, Cmd/Ctrl+hover | jump to the declaration | link underline with hand cursor on hover, chooser popup for several targets, usages popup when already on the declaration | single target, several, none ("Cannot find declaration to go to") |
| S-PI07 | Show Usages popup | Cmd+Alt+F7 / Ctrl+Alt+F7, Cmd+B on a declaration | pick a usage | rows with file, line, code preview; header with count and scope | loading, results, none |
| S-PI08 | Find tool window, usages view | Alt+F7 | browse all usages | tree grouped by directory/file, preview | searching, results |
| S-PI09 | Intention actions popup | Alt+Enter, bulb at the start of the caret line | fix or change code | quick fixes with a bulb icon, a separator, intentions without icon sorted by name, `⋮` on the selected row, submenu arrow (ruff: Disable for this file / this line / this project), footer "Press F1 to toggle preview", speed search | quick fixes first, intentions, none |
| S-PI10 | Rename | Shift+F6 | rename a symbol | in-place rename for local symbols, dialog with options for others | in progress, conflicts |
| S-PI11 | Language Services widget and popup | status bar right of the caret position | server status | icon of the services on the current file (`{}` when none), tooltip `Language Services running on the current file: - Ruff 0.15.20`; popup "Language Services" with "Running on Current File" / "Running on Other Files", rows `ruff 0.15.20`, inline Restart Server / Stop and Autorun Server as Needed / Open Settings / Open Server Error Output | hidden (no services), initializing, running, stopped, terminated (error mark) |
| S-PI12 | Type Engine widget and popup | status bar `Types: Built-in` (Pro only, observed) | choose the type engine | tooltip "Selected type provider for python: Built-in"; popup "Type Engine": `✓ Built-in`, "Install and Use Pyrefly" with a Beta pill, separator, "Type Engine Settings..." | single-module project only |
| S-PI13 | Settings > Python > Tools > External Tools | Settings, or a widget row | enable and locate tools | table: ON/OFF toggle, Tool (+ feature summary, Configure gear), Lookup chain with a mark per stage (`Sdk ✗ · Path ✓ · uvx ✓`), Executable Path (italic grey path or "Not found in $PATH"; install / upgrade / browse); uv footer; "Configure {tool}" dialog with feature check boxes | tool found, not found, uv missing |
| S-PI14 | Settings > Python > Type Engine | Settings, Types popup | choose the engine | segmented button Built-in / Pyrefly (/ ty), description, Pyrefly "Other Features" | multi-module (info only) |
| S-PI15 | Problems tool window | Cmd+6 / Alt+6, inspection widget | list problems of the current file / project | tabs File / Project Errors, rows with severity icon, message, `source(code)`, line | empty, problems |
| S-PI16 | Server error output tab | Language Services row of a terminated server | read stderr of a crashed server | editor tab "{name} Server Error Output" | - |
| S-PI17 | Balloons | server restart or stop, `window/showMessage` | notify | "Ruff server restarted", "Ruff server stopped" / "The server will restart automatically when needed", "{server}: {message}" | - |

### Flows

```
F-PI01 Language services start on a Python file
    open a .py file -> enabled tools for its module start (Initializing),
    S-PI11 shows their icon; diagnostics appear in S-PI01/S-PI02 after the
    300 ms highlighting delay plus server time
    edge: tool missing everywhere and no uv -> nothing starts, no balloon;
          missing tool with uv -> `uvx <tool>` (downloads on first use);
          untrusted project -> nothing starts; more than 10 servers

F-PI02 Complete code
    type letters or `.` -> S-PI03 opens below the caret, first row
    selected -> Up/Down -> Enter inserts, Tab replaces the identifier;
    functions get parentheses (Python insert handler); auto-import items
    add the import line; Escape closes
    happy path keys: 2 (letter, Enter)
    edge: typing narrows without a new request until the list is
          incomplete; inside strings and comments no auto-popup; the
          server answers late (stale answer dropped); thousands of items

F-PI03 Read documentation and signatures
    F1 / Ctrl+J or hover 500 ms -> S-PI04; inside call parentheses Cmd+P ->
    S-PI05, the current parameter follows the caret
    edge: no docs; very long docs (scroll); several overloads

F-PI04 Navigate to a declaration and back
    Cmd+B or Cmd+click on a reference -> declaration opened (other file,
    library stub read-only); on the declaration itself -> S-PI07 with the
    usages; several targets -> chooser; Back returns
    edge: no target ("Cannot find declaration to go to"); stub files in a
          cache directory; target in an excluded folder

F-PI05 Fix a problem
    F2 jumps to the next problem (errors first), hover shows S-PI01
    tooltip -> Alt+Enter -> S-PI09 -> quick fix applied as one undo step;
    ruff submenu: Disable for this file (`# ruff: noqa: CODE` on line 1),
    for this line (`# noqa: CODE`), for this project (`ignore` in
    `pyproject.toml` / `ruff.toml`)
    edge: fix-all, organize imports offered as intentions on any line

F-PI06 Reformat and optimize imports
    Cmd+Alt+L -> whole file (or selection) formatted by ruff when its
    Formatting feature is on, otherwise by the IDE formatter;
    Ctrl+Alt+O -> ruff organize imports (sorts; unused imports stay)

F-PI07 Manage a service
    S-PI11 popup -> Restart Server -> S-PI17 balloon, row shows
    Initializing then the version; crash -> row "| Terminated" with error
    mark, Open Server Error Output -> S-PI16; no automatic restart

F-PI08 Choose tools and the type engine
    S-PI13: enable a tool, pick Lookup, Apply -> servers start or stop;
    S-PI12 / S-PI14: Built-in or Pyrefly (installs `pyrefly>=0.60.0` into
    the interpreter) -> type servers restart, open files re-analysed

F-PI09 Find usages and rename
    Alt+F7 -> S-PI08; Shift+F6 -> S-PI10 -> all references renamed as one
    undo step
```

### Components

| component | variants | states | used on |
|---|---|---|---|
| Problem highlight | error, warning, weak warning, unused, deprecated, information | stale, current | S-PI01 |
| Stripe mark | per severity | hover tooltip | S-PI02 |
| Lookup list | completion | loading, selected row, matched characters | S-PI03 |
| Caret popup | documentation, parameter info, error tooltip | loading, content, scrollable | S-PI04, S-PI05, S-PI01 |
| Link highlight | Cmd/Ctrl hover | hand cursor | S-PI06 |
| List popup with submenus | intentions, usages, declaration chooser | speed search | S-PI06, S-PI07, S-PI09 |
| Status widget | icon list with error overlay, text | hidden, normal, error | S-PI11, S-PI12 |
| Balloon | info | auto-hide | S-PI17 |

### Inferred data model

```
ToolEntry       per tool (ruff, ty, pyright, basedpyright, pyrefly, black):
                enabled (false), discoveryMode INTERPRETER | PATH | UVX
                (INTERPRETER), customPathToExecutable
                stored: .idea/pyLspTools.xml, component PyToolsState
                (older format: enabled / executableDiscoveryMode inside
                RuffConfiguration etc., migrated once)
                evidence: source 11 PyToolsState, ExecutableDiscoveryMode
                confidence: high
ToolFeatures    per tool: inspections, completions, inlayHints,
                documentation, formatting, sortImports (ruff)
                evidence: source 11 RuffConfigurable and peers  confidence: high
TypeEngine      Built-in | Pyrefly | ty, per project, .idea/pyTypeEngine.xml
                evidence: source 11 PyTypeEngineProjectSettings  confidence: high
ServerClient    one per tool and module: command line, roots (module
                content roots), state Initializing | Running | Stopped |
                Terminated, server name and version, stderr tail
                evidence: source 11 LspClientManagerImpl, lsWidget  confidence: high
Diagnostic      range, severity, message, source, code (rule name for
                ruff), tags (unnecessary, deprecated), document version
                evidence: source 11 LspDiagnosticsCustomizer, ruff docs  confidence: high
```

Pyoxide mapping: tool and engine settings in `.oxide/project.toml`
(shared) with personal executable paths in `.oxide/workspace.toml`
(ADR 0009), imported once from `.idea/pyLspTools.xml` and
`.idea/pyTypeEngine.xml`; the rest is runtime state of the language
services crates (ADR 0010).

### Feature matrix
See `replica/features/`, area `python-intel`.

### Out of scope for this area
- PyCharm's own PSI engine and its inspections (replaced by servers).
- Black as an external formatter, notebooks over LSP, Django template and
  framework-specific intelligence, AI completion.
- The reference's master-only additions not in 2026.2.3 (ruff fix on save
  or commit, sort imports when formatting, Zuban).

### Size
Screens 17, flows 9, entities 5. Hard parts: a fast LSP client with
correct incremental sync and position encodings, two servers per file
without duplicate problems, caret popups that leave the focus in the
editor, Markdown rendering, keeping typing latency with server traffic.
Size: L (first version about 60 agent-hours; usages, rename, Problems
window and settings pages about 40 more).

---

## Area: terminal (integrated terminal)

Scope: the Terminal tool window in the reference's default engine: local
shell sessions in tabs over a pseudo-terminal, a VT/xterm emulator on a
character grid, shell integration, activation of the project's virtual
environment, links, find, key routing between the shell and the IDE, and
the settings page. Remote and SSH sessions, AI agent sessions and the
engine choice are not part of it.
Date: 2026-10-16. Reference: PyCharm 2026.2.3 (IntelliJ Community tag
`pycharm/2026.2.3`).

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Terminal | https://www.jetbrains.com/help/pycharm/terminal-emulator.html | tool window, tabs, engines, shell integration, keys |
| 2 | Terminal settings | https://www.jetbrains.com/help/pycharm/settings-tools-terminal.html | options and defaults |
| 3 | IntelliJ Community at `pycharm/2026.2.3`, Apache-2.0 | https://github.com/JetBrains/intellij-community/tree/pycharm/2026.2.3 | `plugins/terminal` (Reworked view, shell start rules, integration scripts, options, actions, bundles), `python/terminal` (virtualenv activation), `platform/execution-impl` (tab titles), keymaps in `platform/platform-resources/src/keymaps/`, colour keys in `platform/platform-resources/src/DefaultColorSchemesManager.xml` |
| 4 | xterm control sequences | https://invisible-island.net/xterm/ctlseqs/ctlseqs.html | modes, mouse reporting, bracketed paste, OSC 0/2/7/8 |
| 5 | Windows pseudoconsole | https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session | ConPTY, Windows 10 1809 or later |
| 6 | `alacritty_terminal` | https://docs.rs/alacritty_terminal | candidate emulator and PTY crate (Apache-2.0) |

No reference screenshots yet: header, menus, command separators, the find
bar and the settings page are to verify against a reference screenshot.

### How the reference is built
- The default engine is "Reworked 2025" (since 2025.2); "Classic" is the
  alternative. The clone has the Reworked look and behaviour, no choice.
- The view draws an emulator core as a character grid (wide characters
  take two cells, line height 1.0); the alternate screen has its own view.
- Shell integration (zsh, bash, PowerShell) reports prompt, command and
  working directory through private OSC sequences and drives separators,
  tab titles and directory tracking; the clone uses the standard OSC 7 and
  OSC 133 sequences with its own scripts.

### Core loop
Press Alt+F12, get a shell in the project root with the project's virtual
environment active, run commands, open a file from a path in the output,
keep several sessions in tabs.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-TM01 | Terminal tool window | Option+F12 / Alt+F12, stripe button (left stripe, bottom group), Open In > Terminal | host the sessions | header: title "Terminal", content tabs, "+" (New Tab) and a chevron (New Predefined Session) right after the tabs, ⋮ Options, Hide; bottom anchor, a third of the window high by default | no tab (showing it creates one), one tab, many tabs |
| S-TM02 | Session view | a tab | the shell | character grid, block cursor, selection, vertical scrollbar, 6 px insets at the top and bottom; alternate screen without scrollbars | focused, unfocused (hollow cursor), alternate screen, session ended |
| S-TM03 | Command separators | each finished command, shell integration on | split the output by command | a gap with a 1 px line; a red marker on the left of a command that exited non-zero | success, error |
| S-TM04 | Find bar | Cmd+F / Ctrl+F in a session | search the output | overlay at the top right (500 px): field, match count, up, down, Match Case, Regex, close; matches yellow, the current one orange | empty, matches, no match |
| S-TM05 | Rename balloon | double-click a tab, Rename Session | rename a session | balloon pointing at the tab: bold "Session name:", field with the text selected | valid, empty (red outline) |
| S-TM06 | New Predefined Session popup | chevron next to "+" | start another shell | detected shells (`name (path)` when two share a name; WSL distributions on Windows), separator, "Settings" | - |
| S-TM07 | Tab context menu | right-click a tab | manage tabs | Rename Session, Move to Editor; Close Tab, Close All Tabs, Close Other Tabs; Split Right, Split Down (and move), Unsplit; Select Next Tab, Select Previous Tab, Show List of Tabs | - |
| S-TM08 | Output context menu | right-click in a session | edit actions | Find, New Tab, Close Tab; Copy, Paste, Paste from History…, Select All; Clear Terminal, Page Up, Page Down, Line Up, Line Down; link actions over a link | selection, no selection, on a link |
| S-TM09 | Options menu | ⋮ in the header | settings | Settings, then the standard tool window items (engine choice, AI agent and feedback items are not cloned) | - |
| S-TM10 | Close confirmation | close a tab whose shell runs a child process | protect running work | "Process 'Terminal Local' Is Running" (the tab name after "Terminal "), "Do you want to terminate the process 'Terminal Local'?", Terminate, Cancel, "Don't ask again" | - |
| S-TM11 | Link hint | plain click on a file path link | teach Cmd/Ctrl+click | after 180 ms a hint at the click: "Open" with "⌘Click" / "Ctrl+Click" | - |
| S-TM12 | Settings > Tools > Terminal | Settings, ⋮ > Settings | options | Project Settings: Start directory, Environment variables; Font Settings: font, fallback, size, line height 1.0, column width 1.0, Configure colors…; Application Settings: Shell path, Default tab name "Local", application title as tab name (on), minimum contrast 4.5, separators (on), Audible bell (on), Close session when it ends (on), Mouse reporting (on), copy on selection (off), middle-click paste (on), Override IDE shortcuts (on), Shell integration (on), Highlight hyperlinks (on), Option as Meta (off), Activate virtualenv (on), cursor shape Block | defaults, modified |

Colours: theme keys `BLOCK_TERMINAL_*` (default colours, ANSI 0 to 15 with
separate text and cell background values, search matches, separators),
falling back to the console colours; the scheme's console font; a minimum
contrast ratio of 4.5.

### Flows

```
F-TM01 Open the terminal
    Alt+F12 -> S-TM01, focus in S-TM02, a tab "Local" created when there is
    none (then "Local (2)", "Local (3)"); Alt+F12 while focused hides it
    edge: last tab closed (a fresh "Local"); untrusted project (no restore)

F-TM02 Start a shell in the project
    shell from Settings, else PowerShell (Windows) or the login $SHELL;
    interactive (and login on macOS) with the integration script; working
    directory: the requested folder, "Start directory", the current file's
    content root, the project root, home; TERM=xterm-256color
    edge: shell path missing; cmd.exe (no integration); no ConPTY; rc files
          that change ZDOTDIR or use `set -u`

F-TM03 Activate the virtual environment
    the interpreter of the module owning the working directory; a venv
    (uv's too) is activated by its own script after the user's rc files,
    before the first prompt, so the prompt shows its prefix
    edge: system interpreter (nothing); interpreter changed while a tab runs
          (balloon "Terminal: project environment changed"); cmd.exe; conda

F-TM04 Work with tabs
    "+" or Cmd+T / Ctrl+Shift+T -> new tab; chevron -> S-TM06; Open In >
    Terminal -> a tab in the file's folder; Cmd+W / Ctrl+F4 -> S-TM10 when a
    child runs; `exit` closes the tab, the last one hides the window;
    double-click -> S-TM05; a running command's title replaces the name
    (cut to 30 characters) unless the user named the tab
    edge: empty name refused; a folder dropped on the tabs opens a tab there

F-TM05 Select, copy, paste, clear
    drag, double and triple click select; Copy needs a selection (Ctrl+C
    without one goes to the shell); bracketed paste; middle click pastes;
    TUI applications get the mouse, Shift selects; Cmd+K clears the
    scrollback and keeps the prompt line
    edge: pasted lines do not run; copy on selection (off)

F-TM06 Follow links
    URLs and Python tracebacks are visible links; file paths (absolute or
    relative to the tracked folder, with :line:column) and OSC 8 links show
    on Cmd/Ctrl+hover and open with Cmd/Ctrl+click; a plain click -> S-TM11
    edge: deleted file; relative path before the folder is known

F-TM07 Find
    Cmd+F / Ctrl+F -> S-TM04 with the selection; Enter or F3 / Cmd+G next,
    with Shift previous; Escape closes it, otherwise Escape goes to the shell

F-TM08 Keys go to the shell
    "Override IDE shortcuts" (on): every key goes to the shell except
    allowed IDE actions (tool windows, window and tab switching, Find in
    Files, Go to Action / File / Class / Symbol, Settings, terminal actions)
    edge: Ctrl+E (Windows, Linux) goes to the shell; Option as Meta; IME
```

Keys (macOS / Windows and Linux): New Tab Cmd+T / Ctrl+Shift+T; Close Tab
Cmd+W / Ctrl+F4; Copy, Paste Cmd+C, Cmd+V / Ctrl+C, Ctrl+V; Clear Cmd+K;
Line Up and Down Cmd+Up, Cmd+Down / Ctrl+Up, Ctrl+Down; Page Up and Down
Shift+PgUp, Shift+PgDn; next and previous tab Cmd+Shift+], [ / Alt+Right,
Alt+Left; Hide Shift+Escape.

### Components

| component | variants | states | used on |
|---|---|---|---|
| Tool window header with content tabs | terminal (shared with Run and Debug) | idle, hover, selected | S-TM01 |
| Terminal grid | normal screen, alternate screen | focused, unfocused, selection, find matches, hovered link | S-TM02, S-TM03 |
| Search overlay | find bar | empty, matches, no match | S-TM04 |
| Balloon with a field | rename | valid, error | S-TM05 |
| Menu, dialog, hint | popups, context menus, terminate process, link hint | - | S-TM06 to S-TM11 |

### Inferred data model

```
TerminalTab      name, user-named flag, shell command, last working
                 directory, environment; order and selected tab
                 stored: the IDE's own workspace file (not in .idea)
                 evidence: source 3                      confidence: high
Session          PTY, shell process, grid, scrollback (1024 KB of output in
                 the reference), alternate screen, modes, title, working
                 directory, command marks with exit codes
                 evidence: sources 1, 3                  confidence: high
TerminalOptions  application (terminal.xml, terminal-local.xml,
                 terminal-font.xml): shell path, tab name, toggles of
                 S-TM12, cursor shape, font; project: start directory,
                 environment variables (workspace.xml), Activate virtualenv
                 (.idea/python-terminal.xml)
                 evidence: sources 2, 3                  confidence: high
```

Pyoxide mapping: application options in the application settings store;
project options and tabs in `.oxide/workspace.toml` (ADR 0009), imported
once from `.idea`. Emulator and PTY: ADR 0014 (proposed; candidate
`alacritty_terminal`, a grid element of its own); venv activation through
a product hook (ADR 0005).

### Feature matrix
See `replica/features/`, area `terminal`.

### Out of scope or deferred
- Engine choice, the Classic engine, AI agent sessions, SSH and remote
  sessions.
- Later: split tabs, Move to Editor, command completion, Paste from
  History, command history search, conda activation.
- Zed's terminal crates are GPL and are not read.

### Size
Screens 12, flows 8, entities 3. Hard parts: key routing (the terminal
wins except for the allowed IDE actions), an exact cell grid with wide
characters and fallback fonts, flood output that leaves typing smooth,
ConPTY on Windows, shell integration that survives users' rc files, IME.
Size: L (first version about 25 to 30 agent-hours; persistence, predefined
sessions, separators, context menus and the settings page about 10 more).

---

## Area: run-debug (debugger)

The run part of this area (Run tool window, run widget, run
configurations: #45, #74) is still to be written here; its scenarios are
in `tests/scenarios/run-debug/`.

Scope: debugging Python programs with the reference's debugger UI: Debug
for the current file and run configurations, line breakpoints in the
gutter with their popup and dialog, exception breakpoints, the Debug tool
window (toolbar, frames, variables, watches, console), stepping,
evaluation, the execution point and inline values in the editor. Pyoxide
debugs through debugpy over the Debug Adapter Protocol (DAP).
Date: 2026-10-16. Reference: PyCharm 2026.2.3 (IntelliJ Community tag
`pycharm/2026.2.3`).

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Debug tool window | https://www.jetbrains.com/help/pycharm/debug-tool-window.html | layout, toolbar, frames, console |
| 2 | Variables | https://www.jetbrains.com/help/pycharm/debug-tool-window-variables.html | variables, watches, evaluate field |
| 3 | Breakpoints | https://www.jetbrains.com/help/pycharm/using-breakpoints.html | types, gutter, properties, Breakpoints dialog |
| 4 | Debug code | https://www.jetbrains.com/help/pycharm/debugging-code.html | stepping, run to cursor, evaluate |
| 5 | Debugger settings | https://www.jetbrains.com/help/pycharm/settings-debugger.html, https://www.jetbrains.com/help/pycharm/settings-debugger-data-views.html, https://www.jetbrains.com/help/pycharm/python-debugger.html | options and defaults |
| 6 | IntelliJ Community at `pycharm/2026.2.3`, Apache-2.0 | https://github.com/JetBrains/intellij-community/tree/pycharm/2026.2.3 | `platform/xdebugger-impl` (session tab, frames, variables, watches, evaluate dialog, breakpoints dialog and popup, inline values, execution point), `platform/xdebugger-api` (bundle, colour keys, breakpoint types), `python/src/com/jetbrains/python/debugger` (Python breakpoints, settings, frames), `python/helpers/debugpy`, `platform/platform-resources/src/idea/{ExecutionActions,LangActions}.xml`, keymaps, icons in `platform/icons/src/expui/{run,debugger,breakpoints}` |
| 7 | Debug Adapter Protocol | https://microsoft.github.io/debug-adapter-protocol/specification | protocol |
| 8 | debugpy | https://github.com/microsoft/debugpy, https://github.com/microsoft/debugpy/wiki/Debug-configuration-settings, https://pypi.org/project/debugpy/1.8.21/ | MIT; launch settings; 1.8.21 supports Python 3.8 and later |

No reference screenshots yet: toolbar order, the console's first lines,
gutter icons and the run widget states are to verify against a reference
screenshot.

### How the reference is built
- Two Python debugger backends share one UI: pydevd (Community) and
  debugpy, the default when available; debugpy comes with a separate
  plugin that is not in Community and falls back to pydevd for remote
  interpreters and Python before 3.9. With it the Debug tool window has a
  "Mode: debugpy | pydevd" switch. The bundled debugpy is 1.8.21.
- Pyoxide has one backend, debugpy 1.8.21 shipped with the application and
  driven over DAP, so no Mode switch; the UI follows the reference.

### Core loop
Click the gutter to set a breakpoint, press Debug, stop on the line, look
at the frames and variables, step, evaluate an expression, resume.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-DG01 | Debug tool window | Debug, Cmd+5 / Alt+5, stripe button | host debug sessions | one tab per session named after the configuration (closing it stops the session); toolbar above the sub-tabs "Threads & Variables" and "Console"; layout menu at the top right (show Frames / Variables, Separate Watches, Show Tab Labels, Restore Default Layout) | running, suspended, finished |
| S-DG02 | Debug toolbar | S-DG01 | control the session | Rerun, Stop; Resume Program, Pause Program, Step Over, Step Into, Step Into My Code, Step Out; View Breakpoints…, Mute Breakpoints, More ⋮ (Force Step Over, Smart Step Into, Run to Cursor, Force Run to Cursor, Show Execution Point, Evaluate Expression…, Debugger Settings ▸, Modify Run Configuration…) | steps disabled while running, Pause disabled while suspended |
| S-DG03 | Threads & Variables | S-DG01 | inspect a stop | Frames on the left, Variables on the right (split 0.35; stacked when docked left or right) | - |
| S-DG04 | Frames | S-DG03 | choose a frame | thread combo on top, Show / Hide Library Frames; rows `func, file.py:12` with a frame icon, frames outside the project greyed, library frames folded; Copy Stack | suspended, "Frames are not available", "Frames not available in non-suspended state" |
| S-DG05 | Variables | S-DG03 | see values | evaluate field on top (history, placeholder "Evaluate expression (Enter) or add a watch (Cmd+Shift+Enter / Ctrl+Shift+Enter)", Add to Watches); watches first, then results, then the frame's variables; rows `name = {type} value` with lazy children ("Collecting data…"); groups "Special Variables", "Return Values"; Simplified Variables View (on) hides functions, classes, modules; menu: New Watch…, Remove Watch, Set Value… (F2), Copy Value, Copy Name, Evaluate Expression…, Add to Watches, Jump To Source (F4) | values, changed after a step, error, "Variables are not available" |
| S-DG06 | Console | S-DG01 | program output and input | the Run console (stdin, ANSI colours, traceback and URL links); "Show Debug Console" turns it into a prompt in the current frame (off) | running, finished |
| S-DG07 | Editor during a session | a stop | show where and what | execution line background (top frame) or a second background (other frames), breakpoint line background, inline values after the line end (`name: value`, italic, changed ones in another colour) on the line where the name is bound, value tooltip after 700 ms, Quick Evaluate (Alt+click) | suspended, running |
| S-DG08 | Gutter breakpoints | line numbers of a Python file | set breakpoints | the icon replaces the line number (New UI default); hover over a stoppable line shows the icon, a hand cursor and "Add Breakpoint"; right-click: Add Breakpoint, Add Conditional Breakpoint…, Add Logging Breakpoint…; tooltip `file.py:12` with suspend policy, condition, log, "Right-click to edit" | enabled, disabled, muted, non-suspending, conditional (question badge), verified, invalid, several on a line |
| S-DG09 | Breakpoint properties popup | right-click a breakpoint; Cmd+Shift+F8 on its line | edit one breakpoint | title `file.py:12`, Enabled, Suspend with All / Thread (Python default Thread) and Make Default, Condition (Python expression editor), "More (Cmd+Shift+F8)", Done | - |
| S-DG10 | Breakpoints dialog | View Breakpoints…, Cmd+Shift+F8 / Ctrl+Shift+F8 | manage breakpoints | "Breakpoints": tree by type ("Python Line Breakpoints", "Python Exception Breakpoints" with "Any exception"), add (Python Exception Breakpoint, class chooser), remove; right: Enabled, Suspend, Condition, Log ("Breakpoint hit" message, Stack trace), Evaluate and log, Remove once hit, Disable until hitting another; exceptions: On termination / On raise, Ignore library files; Done | "Select a breakpoint" |
| S-DG11 | Evaluate dialog | Alt+F8 | evaluate | "Evaluate": expression with history (the selection or the word at the caret), "Result:" tree, Code Fragment Mode / Expression Mode, Evaluate (default), Close; size remembered | result, error |
| S-DG12 | Run widget during a session | main toolbar | control from the toolbar | the Run slot shows "Pause '{name}'" while running and "Resume '{name}'" while suspended; Debug becomes Rerun; Stop appears; dropdown rows have an inline "Debug '{name}'" | running, suspended |
| S-DG13 | Debugger settings pages | Settings > Build, Execution, Deployment > Python Debugger, Debugger (Data Views, Stepping) | options | fields in the data model | later |

### Flows

```
F-DG01 Debug the current file or a configuration
    Debug or "Debug '{name}'" -> S-DG01 with a tab, the Console shows the
    command line; without a breakpoint the Console is focused, "Process
    finished with exit code 0", Rerun offered
    edge: interpreter missing or too old for debugpy; file not runnable;
          single-instance configuration already running (the Run "Process
          Is Running" dialog); adapter fails to start

F-DG02 Stop at a breakpoint
    hit -> S-DG01 shown, Threads & Variables selected once, the file opened
    at the execution line, top frame selected, its variables and inline
    values shown; Resume (F9) runs to the next hit
    edge: several threads; library file; lines edited while running (marks
          move); unverified breakpoint (invalid icon); where the focus goes
          (to verify against a reference screenshot)

F-DG03 Step
    over, into (libraries too), into my code, out, run to cursor, force run
    to cursor (breakpoints ignored), show execution point; each stop updates
    frames, variables (changed values coloured) and inline values
    edge: step past the end (session ends); generators, async code

F-DG04 Inspect and evaluate
    expand lazily; F2 edits a value; Copy Value; Enter in the evaluate field
    adds a result, Cmd+Shift+Enter / Ctrl+Shift+Enter a watch (kept per
    configuration); Alt+F8 -> S-DG11 with the selection, Code Fragment Mode
    edge: huge containers (paging); long strings; errors (red); slow repr

F-DG05 Edit a breakpoint
    right-click -> S-DG09 -> condition -> Done -> question badge; Suspend
    off -> non-suspending; Alt+click toggles enabled; Shift+click adds a
    logging breakpoint; a click removes it; Cmd+Shift+F8 -> S-DG10

F-DG06 Stop on an exception
    default "Any exception" On termination: an uncaught exception suspends
    where it was raised; Resume prints the traceback, exit code 1; On raise
    stops at every raise

F-DG07 Stop, rerun, pause, mute
    Stop kills the program and the adapter; Rerun restarts in the same tab;
    Pause suspends all threads; Mute greys every breakpoint until unmuted
```

Keys (macOS / Windows and Linux): Debug Ctrl+D / Shift+F9; Debug the
current file Ctrl+Shift+D / not in the Community default keymap (docs:
Ctrl+Shift+F9; to verify); Resume Cmd+Alt+R, F9 / F9; Step Over F8; Step
Into F7; Step Into My Code Alt+Shift+F7; Step Out Shift+F8; Run to Cursor
Alt+F9; Force Run to Cursor Cmd+Alt+F9 / Ctrl+Alt+F9; Show Execution Point
Alt+F10; Evaluate Alt+F8; Quick Evaluate Cmd+Alt+F8 / Ctrl+Alt+F8; Toggle
Line Breakpoint Cmd+F8 / Ctrl+F8; View Breakpoints Cmd+Shift+F8 /
Ctrl+Shift+F8; Stop Cmd+F2 / Ctrl+F2; Rerun Cmd+R / Ctrl+F5; Debug window
Cmd+5 / Alt+5; Pause: none.

### Components

| component | variants | states | used on |
|---|---|---|---|
| Session tabs with a toolbar | debug (shares the Run tab strip) | running, suspended, finished | S-DG01, S-DG02 |
| Tree with lazy children | variables, watches, evaluation result | collecting, loaded, error, changed | S-DG05, S-DG11 |
| Gutter icon over a line number | breakpoint kinds | hover, enabled, disabled, muted, invalid | S-DG08 |
| Line background and after-line text | execution point, breakpoint line, inline value | current, other frame, changed | S-DG07 |
| Popup or dialog with a form | breakpoint properties, Breakpoints, Evaluate | - | S-DG09 to S-DG11 |

### Inferred data model

```
LineBreakpoint       file, line, enabled, suspend ALL | THREAD | NONE,
                     condition, log message, log stack, log expression,
                     temporary, group, description
                     stored: .idea/workspace.xml, XDebuggerManager
                     (type python-line, 0-based line)
                     evidence: sources 3, 6               confidence: high
ExceptionBreakpoint  class ("Any exception" = BaseException), on termination
                     | on raise, ignore library files; Watch: expression
                     per run configuration (both in workspace.xml)
                     evidence: sources 3, 6               confidence: high
DebugSession         configuration, adapter and program processes, running
                     | suspended | finished, threads, frames, lazy variables
                     evidence: sources 1, 7               confidence: high
DebuggerOptions      project (workspace.xml): attach to subprocesses (on),
                     evaluation timeout 60000 ms; application
                     (debugger.xml): show window on breakpoint (on), inline
                     values (on), value tooltip (700 ms), simplified view
                     evidence: sources 5, 6               confidence: high
```

Pyoxide mapping: breakpoints, watches, mute state and project options in
`.oxide/workspace.toml` (ADR 0009), imported once from
`.idea/workspace.xml` (never written); application options in the
application settings store. debugpy 1.8.21 ships with the application;
the program starts through DAP `runInTerminal` so the Console keeps stdin
like Run. DAP client and debugpy distribution: ADR 0015 (proposed).

### Feature matrix
See `replica/features/`, area `run-debug` (debugger rows).

### Out of scope or deferred
- The Mode switch and pydevd (one backend).
- First version: breakpoints (gutter, conditions, dialog), Frames and
  Variables, inline values, stepping, the Console, run configurations.
- Later: the Evaluate dialog, Quick Evaluate, value tooltip and watches,
  logging details, Smart Step Into, Jump To Cursor, the Debug Console
  prompt, Reset Frame (not in debugpy), dependent breakpoints,
  subprocesses, Attach to Process, remote, Docker and WSL, numpy and
  pandas views, type renderers, settings pages.

### Size
Screens 13, flows 7, entities 5. Hard parts: three processes per session
(adapter, launcher, program) that must all end on Stop, close or crash,
also on Windows; DAP message ordering; gutter icons and inline values in
the editor; paged variable trees with slow repr; function keys on macOS.
Size: L (first version about 36 to 48 agent-hours).

---

## Area: vcs (git and branch widget)

Scope: Git for the project through the `git` executable: repository
detection, file status colours, change markers in the editor gutter, the
Commit tool window with a side-by-side diff viewer, rollback, the version
control widget in the main toolbar with its branches popup, push, update
(pull) and the Git menu. Blame, history, merge conflicts and stash come
later.
Date: 2026-10-16. Reference: PyCharm 2026.2.3 (IntelliJ Community tag
`pycharm/2026.2.3`).

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Commit and push changes | https://www.jetbrains.com/help/pycharm/commit-and-push-changes.html | Commit tool window, amend, options, push |
| 2 | Manage branches | https://www.jetbrains.com/help/pycharm/manage-branches.html | branches popup, checkout, new branch, Smart Checkout |
| 3 | File status highlights | https://www.jetbrains.com/help/pycharm/file-status-highlights.html | colour per status |
| 4 | Undo changes | https://www.jetbrains.com/help/pycharm/undo-changes.html | rollback of files and lines |
| 5 | Sync with a remote | https://www.jetbrains.com/help/pycharm/sync-with-a-remote-repository.html | fetch, pull, Update Project |
| 6 | Enable version control | https://www.jetbrains.com/help/pycharm/enabling-version-control.html | detection, Create Git Repository |
| 7 | Gutter and diff | https://www.jetbrains.com/help/pycharm/editor-gutter.html, https://www.jetbrains.com/help/pycharm/comparing-file-versions.html | change markers, diff viewer |
| 8 | Version Control settings | https://www.jetbrains.com/help/pycharm/settings-version-control.html, https://www.jetbrains.com/help/pycharm/settings-version-control-git.html | options and defaults |
| 9 | IntelliJ Community at `pycharm/2026.2.3`, Apache-2.0 | https://github.com/JetBrains/intellij-community/tree/pycharm/2026.2.3 | `plugins/git4idea` (commands, status, branches popup, toolbar widget, push, update, bundles), `platform/vcs-impl`, `platform/vcs-api`, `platform/dvcs-impl` (Commit tool window, line markers, file status), `platform/diff-impl` (diff viewer), keymaps, schemes `islands/IslandSchemeDark.xml`, `expUI/expUI_{dark,light}Scheme.xml` |
| 10 | Git documentation | https://git-scm.com/docs/git-status, https://git-scm.com/docs/git-for-each-ref, https://git-scm.com/docs/gitrepository-layout | porcelain output, refs, the `.git` layout |

No reference screenshots yet: toolbar sets, the icons of ignored files,
dialogs and branch submenus are to verify against a reference screenshot.

### Core loop
Edit files and see what changed in the tree and the gutter, review a diff,
commit with Cmd+K, create or switch a branch from the toolbar widget, push.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-VC01 | File status colours | Git on for the project | show a file's state | name in the status colour in the project tree, editor tabs, Commit tree, Recent Files and Go to File rows; folders not coloured by default; an unsaved edit of a tracked file counts as modified at once | added, modified, renamed, deleted, unversioned, ignored, conflict |
| S-VC02 | Gutter change markers | edit a tracked file | show changed lines | a 4 px pill next to the line numbers (wider on hover); deleted lines as a short pill on the boundary; a mark per range in the error stripe | added, modified, deleted, whitespace only |
| S-VC03 | Change marker popup | click a marker | act on one change | Previous Change, Next Change, Rollback Lines, Show Diff for Lines, Copy; the base text with word highlights | - |
| S-VC04 | Commit tool window | Cmd+K / Ctrl+K, Cmd+0 / Alt+0, stripe "Commit" (left, under Project) | commit | toolbar (Refresh, Rollback…, Show Diff, View Options, expand and collapse); changes tree with check boxes: "Changes" (bold) and a grey count "3 files", rows with icon, name in status colour and grey path, "Unversioned Files"; Amend and the message history button; legend "2 modified, 1 added, 1 deleted"; message editor ("Commit Message"); error line; Commit (default), Commit and Push…, gear | empty, changes, updating, committing, amend |
| S-VC05 | Commit options popup | gear, Ctrl+O | options | Author (previous authors), Sign-off commit, Run Git hooks (when hooks exist) | - |
| S-VC06 | Diff viewer | double-click, Cmd+D / Ctrl+D or Enter in S-VC04; Show Diff for Lines | review a change | editor tab "Commit: <file>"; side by side: base on the left (short hash title, read-only), "Your version" on the right (editable); line and word highlights, divider, synchronized scrolling, "{n} differences"; unchanged fragments collapsed with 4 lines of context | differences, "No differences" |
| S-VC07 | Rollback Changes dialog | Cmd+Alt+Z / Ctrl+Alt+Z in S-VC04 | undo file changes | tree of the selection with check boxes, "Delete local copies of added files", Rollback | - |
| S-VC08 | Push dialog | Cmd+Shift+K / Ctrl+Shift+K, Commit and Push… | push | "Push Commits to <repo>": `<branch> → <remote>:<target>` (editable target, "New" tag), commits with subjects, files of the selected commit; Push tags; Push with Force Push (with lease); "Push Rejected" dialog: Merge, Rebase, Cancel | up to date, commits, new branch, rejected |
| S-VC09 | Update Project dialog | Cmd+T / Ctrl+T | pull | merge or rebase incoming changes, clean the working tree with stash or shelve, "Don't show again" | - |
| S-VC10 | VCS widget | main toolbar, right of the project widget | show the branch | branch icon and name (cut in the middle at 80 characters), incoming / outgoing arrows; detached HEAD: warning icon, short hash or tag; "Rebasing {b}", "Merging {b}"; tooltip "Git Branch: {b}"; without VCS "Version Control" without an icon | branch, ahead / behind, detached, operation, no VCS, loading ("Git") |
| S-VC11 | Branches popup | click S-VC10 | branch actions | search "Search for branches and actions" with Fetch and a settings menu; Update Project…, Commit…, Push…; New Branch…, Checkout Tag or Revision…; groups Recent, Local, Remote, Tags, folders for `a/b` names; row: icon (current, favourite star), name, incoming / outgoing counts, grey tracked branch, submenu arrow | git, no VCS (Create Git Repository…) |
| S-VC12 | Branch submenu | a branch row | act on a branch | Checkout, New Branch from '{b}'…, Checkout and Rebase onto '{current}'; Compare with '{b}', Show Diff with Working Tree; Rebase, Merge; Update, Push…, Tracked Branch; Rename… (F2), Delete; items per current / local / remote branch to verify against a reference screenshot | current, local, remote |
| S-VC13 | Branch dialogs | New Branch…; checkout over local changes | create, check out | "Create New Branch": "Branch Name:", "Checkout branch" (on), "Overwrite existing branch"; errors for an empty, existing or invalid name and HEAD; "Git Checkout Problem": Smart Checkout, Force Checkout, Don't Checkout | valid, error |
| S-VC14 | Git menu, VCS Operations popup | main menu, Ctrl+V / Alt+` | all actions | Commit…, Push…, Update Project…, Pull…, Fetch; Merge…, Rebase…; Branches…, New Branch…, New Tag…, Reset HEAD…; Show Git Log; Manage Remotes…, Clone…; without Git the menu is "VCS" | git, no VCS |
| S-VC15 | Balloons | after an operation | report | "{n} files committed" with the message, "Commit failed with error(s)", "Pushed {n} commits to {b}", "Everything is up to date", "Checked out {b}", "Deleted branch {b}" with Restore | success, error |

Status colours (`FILESTATUS_*` keys; Islands Dark / Light): added
`73BD79` / `067D17`; modified and renamed `70AEFF` / `0033B3`; deleted
`6F737A` / `6C707E`; unversioned `E88F89` / `B23247`; ignored `D69A6B` /
`8C4F00` (everything below an ignored folder too; excluded folders keep
their style); conflict `DE6A66` / `DE1B2E`. Gutter `*_LINES_COLOR`, diff
`DIFF_INSERTED`, `DIFF_DELETED`, `DIFF_MODIFIED`.

### Flows

```
F-VC01 Open a Git work tree
    .git found in a content root or above -> S-VC10 shows the branch,
    S-VC01 colours appear; no Git: "Version Control" -> Create Git
    Repository… -> git init -> files red, the default branch shown
    edge: git missing ("Git is not installed"); "Unsupported Git Version"
          (below 2.17, 2.19.2 on Windows); macOS without command line tools
          (the /usr/bin/git shim is not run); project mapped to no VCS

F-VC02 See changes while editing
    type in a tracked file -> name blue at once, gutter marker; new file
    red; `git add` outside -> green within about 0.5 s; .gitignore edits
    update the ignored colours

F-VC03 Commit
    Cmd+K -> S-VC04, changes checked, focus in the message -> type ->
    Cmd+Enter / Ctrl+Enter -> balloon "{n} files committed" (3 keys);
    Amend loads the last message, unticking restores the draft
    edge: nothing checked or empty message (red "Select files to commit",
          "Specify commit message"); unversioned file checked (added);
          user name unset ("Git User Name Is Not Defined"); detached HEAD
          warning; failing hooks; other staged changes stay staged

F-VC04 Review and roll back
    double-click -> S-VC06 -> F7 / Shift+F7 -> edit on the right -> markers
    follow; Cmd+Alt+Z -> S-VC07 -> HEAD content (an added file becomes
    unversioned or is deleted); marker -> S-VC03 -> Rollback Lines (undoable)

F-VC05 Branches
    S-VC10 -> New Branch… -> name -> Create -> widget shows it; Checkout
    edge: invalid or existing name; local changes in the way (S-VC13, Smart
          Checkout stashes and restores); remote branch (a tracking local
          branch); tag or revision (detached HEAD); branch deleted outside

F-VC06 Push, update
    Cmd+Shift+K (or Commit and Push…) -> S-VC08 -> Push -> balloon; a new
    branch gets its upstream; rejected -> Merge or Rebase; Cmd+T -> S-VC09
    -> stash, fetch, merge or rebase, unstash -> "{n} files updated in {m}
    commits" or "All files are up to date"
    edge: authentication (no prompt yet: git's message); no tracked branch;
          conflicts (error, repository left merging or rebasing)
```

Keys (macOS / Windows and Linux): Commit… Cmd+K / Ctrl+K; Commit window
Cmd+0 / Alt+0; Commit and Push… Cmd+Alt+K / Ctrl+Alt+K; Amend Ctrl+Alt+M
/ Alt+M; Rollback Cmd+Alt+Z / Ctrl+Alt+Z; Show Diff Cmd+D / Ctrl+D; Push…
Cmd+Shift+K / Ctrl+Shift+K; Update Project… Cmd+T / Ctrl+T; next change
Ctrl+Alt+Shift+Down; next difference F7; New Branch… Cmd+Alt+N /
Ctrl+Alt+N; VCS Operations Ctrl+V / Alt+`.

### Components

| component | variants | states | used on |
|---|---|---|---|
| Status-coloured file name | tree row, tab, list row | per status | S-VC01, S-VC04 |
| Gutter change marker | added, modified, deleted, whitespace | idle, hover | S-VC02 |
| Changes tree with check boxes | changes, unversioned | included, partly included, updating | S-VC04, S-VC07 |
| Diff panes | side by side (unified later) | differences, none | S-VC06 |
| Toolbar widget and branch menu | VCS widget, branches popup with groups and submenus | branch, detached, no VCS, search | S-VC10 to S-VC12 |
| Dialog, balloon | rollback, push, update, branch, user name; info and error balloons | valid, error | S-VC07 to S-VC15 |

### Inferred data model

```
Repository   root, git dir (`gitdir:` for worktrees), HEAD (branch or
             detached), state (merging, rebasing, ...), remotes, branches
             with upstream and ahead / behind; read from .git files (HEAD,
             refs, packed-refs, config) and `git for-each-ref`
             evidence: sources 2, 9, 10           confidence: high
FileStatus   path -> added | modified | renamed | deleted | unversioned |
             ignored | conflict (`git status --porcelain -z`)
             evidence: sources 3, 9, 10           confidence: high
LineRanges   per open tracked file: base text (`git cat-file --filters
             HEAD:<path>`) against the buffer -> inserted, modified, deleted
             evidence: sources 7, 9               confidence: high
CommitDraft  message, included paths, amend, author, sign-off, run hooks
VcsSettings  mapping root -> Git (shared .idea/vcs.xml); path to git
             (git.xml); update method, recent branches and messages
             (workspace.xml)
             evidence: sources 1, 8, 9            confidence: high
```

Pyoxide mapping: path to git in the application settings store; commit
draft, recent messages and branches in `.oxide/workspace.toml` (ADR 0009);
only `.idea/vcs.xml` is imported once. Every operation runs the `git`
executable as the reference does (ADR 0016, proposed): reads with
`GIT_OPTIONAL_LOCKS=0`, no terminal prompts, the user's credential helper
and ssh-agent, status 300 ms after a watcher batch, never on the UI
thread. Proposed budgets: status of a 10k-file repository within 150 ms of
the watcher batch; gutter markers within 50 ms of the last keystroke.

### Feature matrix
See `replica/features/`, area `vcs`.

### Out of scope or deferred
- Later: blame, Git log tool window and file history, merge conflict UI,
  stash and shelve UI, cherry-pick, interactive rebase, changelists,
  partial commits, compare with a branch, worktrees, tags, a credentials
  prompt, `.gitignore` support, commit checks, hosting services, clone.
- Version control systems other than Git.

### Size
Screens 15, flows 6, entities 5. Hard parts: the status pipeline with the
file watcher (own writes, `.git` events, large repositories) within the
budgets; the diff viewer as a new kind of editor tab; the gutter marker
layer; process cost and line endings on Windows; balloons as a shared
component. Size: L (about 42 agent-hours).

---

## Area: settings (Settings window, keymap, interpreters)

Scope: the Settings window (page tree with search, pages, Revert changes,
Apply / OK / Cancel), Pyoxide's application settings store, the first
pages (Appearance with themes, System Settings for autosave, Keymap,
Editor > Font, Editor > General soft wraps, Python > Interpreter with the
Add Python Interpreter dialog, Python > Type Engine, Python > Tools >
External Tools) and a one-time pick-up of the reference IDE's settings.
Pages of other areas come with those areas.
Date: 2026-10-16. Reference: PyCharm 2026.2.3 (IntelliJ Community tag
`pycharm/2026.2.3`).

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Settings dialog | https://www.jetbrains.com/help/pycharm/settings-preferences-dialog.html | tree, search, Apply / OK |
| 2 | Appearance | https://www.jetbrains.com/help/pycharm/settings-appearance.html | theme, sync with OS, UI options |
| 3 | System Settings | https://www.jetbrains.com/help/pycharm/system-settings.html | autosave and sync |
| 4 | Keymap | https://www.jetbrains.com/help/pycharm/settings-keymap.html | keymap choice, actions tree, find by shortcut |
| 5 | Editor > Font, Editor > General | https://www.jetbrains.com/help/pycharm/settings-editor-font.html, https://www.jetbrains.com/help/pycharm/settings-editor-general.html | font, line height, soft wraps |
| 6 | File Types | https://www.jetbrains.com/help/pycharm/settings-file-types.html | associations, ignored files |
| 7 | Python interpreter | https://www.jetbrains.com/help/pycharm/configuring-python-interpreter.html, https://www.jetbrains.com/help/pycharm/creating-virtual-environment.html | Interpreter page, Add Python Interpreter |
| 8 | Share and import settings | https://www.jetbrains.com/help/pycharm/sharing-your-ide-settings.html | import, first run |
| 9 | IntelliJ Community at `pycharm/2026.2.3`, Apache-2.0 | https://github.com/JetBrains/intellij-community/tree/pycharm/2026.2.3 | `platform/platform-impl` (Settings window: `SettingsEditor`, `SettingsTreeView`, `SettingsFilter`, `SettingsNonModalDialog`, `ConfigurablesListPanel`; `AppearanceConfigurable`, `GeneralSettingsConfigurable`, `KeymapPanel`, `QuickChangeLookAndFeel`), `platform/platform-resources` (advanced settings, keymaps), `plugins/keymaps/vscode-keymap` (VS Code keymaps), `python/` (`PyActiveSdkConfigurable`, `PythonAddCustomInterpreter`, the Python settings group) |

No reference screenshots yet: tree row height and order, the banner, theme
combo groups, the gear menu and the Python pages are to verify against a
reference screenshot.

### How the reference is built
- The Settings window is non-modal by default in 2026.2 (advanced setting
  "Show settings in a non-modal dialog", on): one window per application,
  title `Settings – <project>`, with Pin Window; modal from the welcome
  screen.
- The Python pages form a top-level "Python" group (Interpreter, Type
  Engine, Tools > External Tools) sorted first in the tree.
- Autosave lives in Appearance & Behavior > System Settings; the page
  reset link is "Revert changes"; there is no Reset button.
- Theme changes apply at once, not on Apply; the quick switcher previews a
  theme 500 ms after the selection moves and restores it on Escape.

### Core loop
Press Cmd+, / Ctrl+Alt+S, type a few letters to find an option, change it,
Apply; pick a theme or a keymap and see it at once.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-SE01 | Settings window | Cmd+, / Ctrl+Alt+S, toolbar gear > Settings…, macOS app menu Settings… | change settings | left: search field and page tree (top-level rows bold, project-level mark, modified pages in the link colour, errors red, sticky group name); right: banner (bold breadcrumbs "Appearance & Behavior › Appearance", Revert changes, Back, Forward, Pin Window) and the page; bottom bar: help "?", Cancel, Apply, OK (macOS) or OK, Cancel, Apply (Windows, Linux); 900×700 initial and minimum, splitter 0.2, both remembered | page, filtered, no match (red field), modified, error |
| S-SE02 | Group page | a group row | overview | description and links to the child pages | - |
| S-SE03 | Appearance | Appearance & Behavior > Appearance | look | Theme combo, Sync with OS and its gear, editor scheme; later: zoom, UI font, UI options | - |
| S-SE04 | Theme quick switch | gear > Theme ▸, Ctrl+` > Theme | switch themes | themes in groups (Islands, New UI, Classic, "Custom" for imported ones), current marked, separator, "Import Theme…" (in place of the marketplace link) | preview |
| S-SE05 | System Settings | Appearance & Behavior > System Settings | autosave | Files: "Save files if the IDE is idle for [15] seconds", "Save files when switching to a different application or a built-in terminal", "Sync external changes" | - |
| S-SE06 | Keymap | tree, gear > Keymap ▸ | choose and read shortcuts | keymap combo (predefined bold, custom below their parent) and gear (Duplicate, Rename, Restore Defaults, Delete); "Based on {name} keymap"; Expand All, Collapse All, Edit Shortcut; search by name, Find Actions by Shortcut; actions tree with shortcuts | predefined (read-only), custom |
| S-SE07 | Editor > Font | tree | editor font | Font with "Show only monospaced fonts", Size, Line height (1.2), Enable ligatures, Restore defaults (when changed), Typography Settings (weights, fallback font), preview | defaults, changed |
| S-SE08 | Editor > General | tree | editor behaviour | groups Mouse Control, Soft Wraps ("Soft-wrap these files:" `*.md; *.txt; *.rst; *.adoc`, original indent (on), additional indent 0, indicators for the current line only (off)), Scrolling, On Save | - |
| S-SE09 | Editor > File Types | tree | associations | Recognized File Types (types, file name patterns, hashbangs), Ignored Files and Folders | later |
| S-SE10 | Python > Interpreter | tree, interpreter popup "Interpreter Settings…" | project interpreter | "Python Interpreter:" combo (No interpreter, environments, system, separator, "Show All…"), "Add Interpreter" ▸; packages table (Package, Version, Latest version; install, uninstall, upgrade) | none, loading packages, configuring |
| S-SE11 | Python Interpreters | "Show All…" | manage the list | configured interpreters, add, remove, rename, path; "Hide virtual environments associated with other projects" | duplicate name error |
| S-SE12 | Add Python Interpreter | Add Interpreter ▸ Add Local Interpreter…, interpreter popup | add one | "Environment: Generate new / Select existing"; "Type:" Virtualenv, Conda, Pipenv, Poetry, uv, Hatch (new) or Python, Conda, Poetry, uv, Hatch (existing); Base Python, Location, Inherit packages, Python path | tool not found, folder exists |
| S-SE13 | Python > Type Engine | tree | type engine | see S-PI14 | - |
| S-SE14 | Python > Tools > External Tools | tree | language tools | see S-PI13 | - |
| S-SE15 | Unsaved Changes | close the project or quit with edits | keep edits | "Unsaved Changes", Apply, Don't Save, Cancel | - |
| S-SE16 | First run import | first start without settings | carry settings over | "Import Settings From" (the reference's configuration) or "Skip Import"; "Choose Theme"; "Choose Keymap" with sample shortcuts; Back, Continue | later |

Pyoxide's tree lists only implemented pages, in the reference's order:
Python (Interpreter, Type Engine, Tools > External Tools), Appearance &
Behavior (Appearance, System Settings), Keymap, Editor (General, Font, File
Types). The default page is the last one selected, else Appearance.

### Flows

```
F-SE01 Open Settings
    Cmd+, / Ctrl+Alt+S -> S-SE01 on the last page (first time Appearance),
    focus in the tree; again while open: the same window to the front
    edge: opened from another project with unsaved edits -> S-SE15

F-SE02 Search
    typing (the tree forwards it to the field) -> after 100 ms the tree
    keeps pages whose name or options match (word prefixes) and selects the
    first hit; matching labels highlighted; Enter focuses the page
    edge: no hit (red field); Escape clears the filter, a second one closes

F-SE03 Change, Apply, OK, Cancel
    a change -> page name in the modified colour, Revert changes and Apply
    enabled -> Apply saves every modified page; OK = Apply and close;
    Cancel or Escape drop the edits
    edge: Apply error (page selected, name red, message under the page)

F-SE04 Switch the theme
    Theme combo switches at once (Cancel keeps it); gear > Theme ▸ previews
    after 500 ms, Enter keeps, Escape restores; Import Theme… (.theme.json,
    .icls, .xml, .jar, .zip, folder) -> report -> listed under "Custom"
    edge: unknown keys (report); a plugin theme id that cannot be resolved

F-SE05 Switch the keymap
    Keymap combo -> Apply (gear > Keymap ▸ at once) -> bindings and menu
    shortcuts change; "macOS", "Windows", "GNOME", "KDE", "XWin", "VSCode",
    "VSCode (macOS)"; the default follows the platform

F-SE06 Font, soft wraps, autosave
    change -> Apply -> every editor relayouts, matching files wrap, the
    autosave delay applies

F-SE07 Interpreter
    combo -> Apply; Add Interpreter ▸ Add Local Interpreter… -> S-SE12 -> OK
    -> created off the UI thread, added to the application list, selected
    edge: broken base Python; "The specified folder already exists"; "No
          {tool} executable found. Make sure {tool} is installed and added
          to PATH."

F-SE08 First start
    no settings -> S-SE16: import the newest reference configuration or
    skip -> theme -> keymap; later starts do not import again
```

Keys: Settings Cmd+, / Ctrl+Alt+S; Quick Switch Scheme Ctrl+`. Gear menu:
Settings… | Theme ▸, Keymap ▸ (the reference also has Run Anything,
Plugins, View Mode).

### Components

| component | variants | states | used on |
|---|---|---|---|
| Settings tree with search | page, group, project page | selected, modified, error, filtered, no match | S-SE01 |
| Banner | breadcrumbs, Revert changes, history | modified, unmodified | S-SE01 |
| Form rows | label and field, check box, combo, link, group header, collapsible group | enabled, disabled, error | S-SE03 to S-SE14 |
| Bottom bar | per-OS button order | Apply enabled or not | S-SE01, S-SE12 |
| Quick switch popup | theme, keymap | preview | S-SE04 |
| Table | packages, interpreters | loading, empty | S-SE10, S-SE11 |

### Inferred data model

```
ReferenceConfig  the reference's per-OS configuration directory, options/:
                 laf.xml (theme id, absent = Islands Dark), colors.scheme.xml
                 and colors/*.icls (scheme), keymap.xml and keymaps/*.xml
                 (active and custom keymaps), editor-font.xml (family, size,
                 line spacing, ligatures), editor.xml (soft wraps),
                 ui.lnf.xml (UI options), ide.general.xml (autosave),
                 filetypes.xml (associations, ignored files), jdk.table.xml
                 (interpreters: name, home, flavour, associated project)
                 evidence: sources 8, 9               confidence: high
SettingsPage     id, parent, title, weight, scope (application | project
                 shared | project personal), search terms, draft with
                 modified / apply / reset
                 evidence: source 9                   confidence: high
Interpreter      name, path, kind, associated project; an application list,
                 a project refers to one by name
                 evidence: sources 7, 9               confidence: high
```

Pyoxide mapping (ADR 0017, proposed): `<config>/pyoxide/settings.toml`
(versioned, unknown keys kept, atomic writes, a done-once import marker),
growing data in their own files (`interpreters.toml`, themes, keymaps);
project settings in `.oxide/project.toml` and `.oxide/workspace.toml`
(ADR 0009). The import reads only the XML data files above and theme,
scheme and keymap data in plugin archives; reading a plugin descriptor to
resolve a theme id is a decision for that ADR. Keymap format and runtime
switching: ADR 0018 (proposed); VS Code keymaps come from Community data
files (NOTICE entry).

### Feature matrix
See `replica/features/`, area `settings`; import formats stay in area
`settings-import`, the packages table is the python-env row "Package
manager panel".

### Out of scope or deferred
- Pages for features Pyoxide does not have yet; a page comes with its
  feature.
- Later: search spotlight on the page, keymap editing, File Types page,
  the first-run window, other Appearance options.
- A Zed keymap (no permissively licensed source); marketplace links.

### Size
Screens 16, flows 8, entities 3. Hard parts: a non-modal window with
drafts and live apply, runtime keymap switching (menus, gestures), search
highlights on pages, monospaced font detection, theme ids of plugin
themes, several main windows in sync. Size: L (first version about 32 to
39 agent-hours; packages table, File Types and first run about 10 more).

---

## Area: db-gui (Database tool window, SQL console, data grid)

Scope: the user interface on top of the database engine (area `db`, ADR
0002 to 0004): data sources and their dialog, the Database tool window with
the object tree, SQL consoles and SQL files bound to a data source, result
tabs and the output log, the data grid (paging, sorting, filters, value
display), copy and export, import from a file, and later editing of data and
of objects. PostgreSQL, Oracle and SQLite only. Database tools are a
Professional-edition feature of the reference.
Date: 2026-10-22. Reference: PyCharm 2026.2.3 (Professional), observed
behaviour and public documentation; no Community source exists for the
database tools.

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Data sources and drivers dialog | https://www.jetbrains.com/help/pycharm/data-sources-and-drivers-dialog.html | General, Options, SSH/SSL, Schemas, Advanced |
| 2 | Configure connections | https://www.jetbrains.com/help/pycharm/configuring-database-connections.html | test connection, password save modes |
| 3 | Run queries | https://www.jetbrains.com/help/pycharm/run-a-query.html | execute, cancel, what runs at the caret |
| 4 | Rows and paging | https://www.jetbrains.com/help/pycharm/rows.html | page size, counting rows |
| 5 | Sessions | https://www.jetbrains.com/help/pycharm/managing-connection-sessions.html | one session per console, transaction mode |
| 6 | Query results | https://www.jetbrains.com/help/pycharm/viewing-query-results.html | result tabs, output |
| 7 | Database tool window and table editor | https://www.jetbrains.com/help/pycharm/database-tool-window.html | tree, toolbar, view options |
| 8 | Query consoles | https://www.jetbrains.com/help/pycharm/query-consoles.html | consoles are files, history |
| 9 | Export and import data | https://www.jetbrains.com/help/pycharm/export-data.html | dump and import dialogs |
| 10 | PostgreSQL system catalogs and `EXPLAIN` | https://www.postgresql.org/docs/current/catalogs.html | introspection, plans |
| 11 | Observed behaviour of a licensed copy of the reference: what a user sees and what the database server sees (statements in the server log, `pg_stat_activity`) | local notes, not published | page SQL, connection counts, savepoints, row identity; open questions below |
| 12 | DataCraft, MIT OR Apache-2.0, https://github.com/javy-fernandez/datacraft | logic only | see ADR 0021; its UI, theme, icons and strings are not used |

The help pages were listed from the documentation index and their text was
not re-read for this pass. Details marked "verify" need a reference
screenshot or an observation (the DB shot list covers them).

### Core loop
Open the Database tool window, add a data source and connect, expand
tables from the tree, double-click a table to look at its rows, open a
console, run a query with Cmd/Ctrl+Enter and read the result in a grid, then
export what is on screen. Editing rows comes after reading them well.

### Screens

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-DG01 | Database tool window | stripe button on the right, View > Tool Windows | the data sources and their objects | header with toolbar (New, Refresh, Console, Table Editor, Stop, filter, options); tree; speed search; empty state with a link to create a data source | no data source, disconnected, connecting, connected, loading, error node, filtered |
| S-DG02 | Data Sources dialog | New (+) > Data Source, Properties on a node, Cmd/Ctrl+; | define a connection | list of data sources on the left; tabs General (name, host, port, database/service, user, password and save mode, authentication, URL, Test Connection), Options (read-only, transaction mode, keep-alive, auto-disconnect, startup script, time zone), SSH/SSL, Schemas, Advanced | new, edited, testing, test ok (server and driver version), test failed (message and hint), unsaved changes |
| S-DG03 | Tree node context menus | right-click on data source, database, schema, group, table, column, routine | actions on objects | Refresh, New > Console, Open Data, Copy DDL, SQL Scripts, Modify, Drop, Truncate, Properties, Copy Reference, Jump to Source | per node kind; a read-only data source disables the writing items |
| S-DG04 | SQL console tab | New > Console, Jump to Query Console, or a bound `.sql` file | write and run SQL | editor; toolbar: Execute, Stop, data source and schema switchers, transaction mode, Commit, Rollback, History, Explain; run marks in the gutter | idle, running, cancelling, transaction open (pending count), disconnected, syntax error |
| S-DG05 | Statement picker popup | Execute with the caret in nested statements | choose what to run | small list: innermost, outer statement, whole script, from caret | shown, dismissed |
| S-DG06 | Parameters dialog | Execute a statement with `?`, `:name`, `$1` or a `${name}` pattern | ask for values | table of names and values, remembered values, Execute | filled, empty, cancelled |
| S-DG07 | Dangerous statement confirmation | `DELETE` or `UPDATE` without `WHERE` | prevent accidents | message, Execute, Skip, Cancel | shown |
| S-DG08 | Output and results panel | below the console, or a Services-like tool window (verify) | show what each statement did | Output log: one line per statement with echoed schema, rows or affected count, time; result tabs (numbered, pinned, renamed); server notices | empty, one result, many, error with position, running |
| S-DG09 | Data grid | double-click a table, a result tab | read rows | page bar (first, previous, next, last, page size, row range, count), WHERE and ORDER BY fields, column headers with type, PK and FK marks, NULL marker, row numbers, status line | loading, page, last page, empty, error, truncated values, filtered, sorted |
| S-DG10 | Value editor panel | View Value on a cell (verify the key) | read and edit a long cell | text area, format switch (text, JSON, hex), Save to file, Load full value | text, json, binary, truncated |
| S-DG11 | Grid view modes | grid toolbar | other views of the page | transposed record view, text view (ASCII table, CSV...), aggregates of the selection (count, sum, avg, min, max, median) | each mode |
| S-DG12 | Export dialog | grid toolbar Export, tree: Export Data | write rows to a file | format, destination, options per format, "re-run without paging" | CSV, TSV, JSON, SQL INSERT, Markdown, HTML; progress; done |
| S-DG13 | Import from file dialog | table context menu > Import | load a CSV into a table | file, delimiter, header row, quote, NULL text, column mapping table, preview of rows, error policy | mapped, mismatched, running, finished with a report |
| S-DG14 | DDL viewer and SQL scripts | tree: DDL, Copy DDL, SQL Scripts submenu | see and reuse definitions | read-only SQL editor tab or clipboard | table, view, routine, index |
| S-DG15 | Settings > Database | Settings page | defaults of query execution and data views | Query Execution (what runs at the caret, confirm dangerous statements, query timeout), Data Views (page size, fetch size, date and number formats, NULL text, LOB limits), General (introspection, tree options) | defaults, modified |
| S-DG16 | Explain plan | Explain action in the console | plan of a statement | tree with node, cost, rows, time; raw text | plan, analyze, error |
| S-DG17 | Transaction prompts | close a console, disconnect, close the IDE with an open transaction | protect pending work | Commit, Rollback, Cancel | shown |
| S-DG18 | Introspection progress | Refresh or first connect | show background loading | status bar progress with database and schema, cancel | running, cancelled, done |
| S-DG19 (later) | Pending edits in the grid | edit a cell, Add Row, Delete Row | stage changes | changed cell colour, deleted row struck through, new row mark, Submit, Revert, Preview SQL | clean, dirty, submitting, conflict |
| S-DG20 (later) | Modify and Create object dialogs | tree: Modify Table, New > Table | change objects through generated DDL | columns, keys, indexes tabs; SQL preview; Execute | new, modified |

### Flows

```
F-DG01 Add a data source and connect                      (clicks: about 8)
    S-DG01 empty -> New > Data Source > pick the engine -> S-DG02 General:
    host, port, database, user, password -> Test Connection (result in S-DG02)
    -> OK -> connect on first expansion or the Connect button -> tree loads
    edge: wrong password, unreachable host, TLS required, missing Oracle
          client, a SQLite file that does not exist, password mode "ask"

F-DG02 Look at a table                          (clicks: 4 from a loaded tree)
    S-DG01 expand data source > schema > Tables (instant, from the model)
    -> double-click the table -> S-DG09 first page (500 rows) -> sort by a
    header click, type a WHERE condition, Enter -> next page
    edge: table with millions of rows (count on demand), no rows, wide
          values, a permission error, a table dropped meanwhile

F-DG03 Run a query in a console                           (clicks: 3)
    New > Console (S-DG04) -> type SQL -> Cmd/Ctrl+Enter -> S-DG08 line plus
    S-DG09 in a result tab -> run another statement: an unpinned tab is reused
    edge: several statements selected, a function body that contains
          semicolons, a statement without rows, a long query: Stop, two result
          sets, an error with a cursor position

F-DG04 Run a script with an error
    select text -> Execute -> statements run one by one -> one fails ->
    choice: continue, skip, stop -> the rest runs or not
    edge: an error inside a transaction in manual mode (savepoint, S-DG17)

F-DG05 Parameterised and risky queries
    query with ":name" -> S-DG06 -> values remembered next time;
    DELETE without WHERE -> S-DG07

F-DG06 Switch the schema of a console
    schema chip -> list -> the session's search path changes; typing
    "set search_path" by hand updates the chip

F-DG07 Refresh and DDL changes
    tree Refresh (incremental), full refresh, invalidate cache;
    CREATE, ALTER and DROP in a console update the affected nodes alone
    edge: object dropped elsewhere, server restarted, no catalog privileges,
          cancel during refresh keeps the old model

F-DG08 Export what is on screen                           (clicks: 4)
    S-DG09 Export -> S-DG12 -> format and options -> file written; the whole
    result re-run without the page limit; copy of a selection in a format
    edge: huge result (streamed, cancellable), NULL and binary values,
          characters that break spreadsheets

F-DG09 Import a CSV into a table
    table menu > Import -> S-DG13 -> pick the file, adjust delimiter and
    mapping -> run in one transaction -> report
    edge: wrong types, extra columns, encoding, 1 million rows (streamed)

F-DG10 Open DDL, generate SQL
    tree DDL / Copy DDL / SQL Scripts: generated CREATE, SELECT, INSERT,
    UPDATE, DELETE for the selected object into a console or the clipboard

F-DG11 Transactions (manual mode)
    switch mode -> statements stay in a transaction with a visible pending
    indicator -> Commit / Rollback; a failed statement does not poison the
    transaction (rollback to a savepoint); closing asks

F-DG12 (later) Edit rows
    edit a cell, add and delete rows -> pending marks -> Preview SQL ->
    Submit (DML in a fixed order) -> page reloaded; Revert drops the edits
    edge: no primary key (identify by all comparable columns, or by the
          physical row id on PostgreSQL), 0 or 2 rows affected -> warning and
          rollback, read-only data source, computed columns
```

### Components

| component | variants | states | used on |
|---|---|---|---|
| Tool window with toolbar and tree | Database (right stripe) | idle, hover, selected, loading spinner, error | S-DG01 |
| Tree row | data source, database, schema, group, object, column, key, index, routine | collapsed, expanded, selected, filtered, disabled, with badges (PK, FK, NOT NULL), with detail text (type) | S-DG01 |
| Form dialog with a left list | Data Sources | clean, dirty, error | S-DG02 |
| Combo or chip | data source, schema, transaction mode, page size | open, disabled | S-DG04, S-DG09 |
| Code editor with a run gutter | console, bound file | as in area `editor` | S-DG04 |
| Result tabs | pinned, renamed, numbered | selected, running, error | S-DG08 |
| Output log | echo line, status line, notice line, error line with a link | - | S-DG08 |
| Data grid | table, record, text | cell and range selection, editing, pending change, NULL, truncated, sort arrow, column resize and reorder | S-DG09, S-DG11 |
| Page bar | buttons, row range, count link | enabled, disabled, loading | S-DG09 |
| Filter fields with completion | WHERE, ORDER BY | empty, valid, error | S-DG09 |
| Value editor | text, JSON, binary | read-only, editable | S-DG10 |
| Popups and menus | statement picker, context menus, history list | - | S-DG03, S-DG05 |
| Dialogs | parameters, confirmations, export, import | - | S-DG06, S-DG07, S-DG12, S-DG13 |

### Inferred data model

```
DataSource       id (uuid), name, backend (postgres | oracle-thin | oracle-odpi |
                 sqlite), connection (host, port, database | service | sid |
                 dsn | path), user, secret mode (keychain | session | ask), tls,
                 read_only, tx_mode (auto | manual), isolation, keep_alive,
                 auto_disconnect, startup_script, time_zone, introspection
                 scope (databases, schemas, patterns, level per schema);
                 a shared part (may be committed) and a personal part
                 evidence: S-DG02, sources 1, 2, 11     confidence: high
Session          data source, connection, tx state (open, pending count),
                 current schema or search path, running statement, last activity;
                 one per console by default; metadata refresh uses its own
                 short-lived connection
                 evidence: sources 5, 11               confidence: high
Console          id, data source, bound schema, file path (a .sql file),
                 history of executed statements (text, time, duration)
                 evidence: source 8                    confidence: high
SqlFileBinding   file path -> data source, schema
                 evidence: sources 8, 11               confidence: medium
MetadataModel    per data source: a tree of nodes (database > schema > object >
                 children), each with id, kind, name, parent, comment and
                 kind-specific fields (column type, nullability, default,
                 identity; key columns; FK targets and actions; index method,
                 uniqueness), a state number per object for incremental
                 refresh, the loaded level per schema, source text loaded
                 lazily; stored on disk per data source, usable before connecting
                 evidence: S-DG01, sources 7, 11       confidence: high
ResultTab        id, console, title, pinned, renamed, query text, columns,
                 current page, page size, row range, total (known or not),
                 WHERE and ORDER BY text, sort, view mode
                 evidence: S-DG08, S-DG09, source 6    confidence: high
GridPage         rows of typed values (db_core::Value), has_more, truncated cells
                 evidence: source 4                    confidence: high
PendingEdits     changed cells (row key, column, old, new), deleted row keys,
                 new rows; applied in a fixed order (delete, update, insert)
                 evidence: S-DG19, source 11           confidence: medium
ExportJob        source (tab or table), format, options, target, re-run flag
ImportJob        file, delimiter, header, quote, NULL text, column map, policy
                 evidence: S-DG12, S-DG13, source 9    confidence: medium
DbSettings       page size, fetch size, LOB limit, NULL text, date and number
                 formats, confirm dangerous statements, what runs at the caret,
                 query timeout, introspection options
                 evidence: S-DG15                      confidence: medium
```

Pyoxide mapping: `DataSource` is an ADR 0004 profile plus GUI options;
`Session` is a `db_core::Connection`; `GridPage` comes from
`Cursor::next_page`; `MetadataModel` is new (ADR 0021). Console files,
histories and bindings live in the personal project state and the
configuration folder (ADR 0009); the committable part of a data source goes
to `.oxide/` (ADR 0008).

### Feature matrix
See `replica/features/`, areas `db-gui` (this area, one file per feature)
and `database` (the engine).

### Out of scope or deferred
- JDBC drivers and engines beyond PostgreSQL, Oracle and SQLite; SSH
  tunnels (planned after the first version).
- Diagrams, schema comparison, data comparison, test data generation,
  database refactorings, code generation, NoSQL, cloud connectors, AI
  assistant.
- Editing data and objects (S-DG19, S-DG20) come after the read-only
  milestone; they have their own rows.
- Open questions to settle by observation: the keep-alive query and
  interval, the object count at which the tree loads names only, whether a
  page asks for one extra row, the SQL of a submit on a table without a
  key, how many connections several consoles hold, defaults of
  Settings > Database.

### Size
Screens 20, flows 12, entities 10. Hard parts: a virtualized grid that
stays smooth with wide values; a tree on tens of thousands of objects
served from a model; cancellation on every driver (the thin Oracle driver
cannot cancel: it reconnects); a statement splitter that understands dollar
quotes, PL/SQL blocks and comments; incremental metadata refresh on
PostgreSQL; transactions with savepoints; editing rows safely without a
key. Size: XL as a whole. Milestone M6 first version (data sources, tree,
console, read-only grid, export and import): about 190 agent-hours of
work, about 90 on the critical path with three or four agents in parallel;
incremental refresh, editing and object dialogs about 45 more.
