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
See `replica/features.csv`, areas `database` and `db-gui`. Database tools
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
See `replica/features.csv`, area `editor`: text buffer and editing, Python
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
See `replica/features.csv`, area `python-intel`.

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
