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
