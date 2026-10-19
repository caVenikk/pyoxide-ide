# 0007. Application shell and editor

- Status: accepted
- Date: 2026-10-11

## Context
ADR 0006 chose GPUI as the UI toolkit. The first usable window needs the
main window chrome of the reference IDE (main toolbar in the title bar
area, tool window stripes, the project tool window, editor tabs, status
bar), a code editor that stays within docs/performance.md on 100k-line
files, Python highlighting, and a command for every user action (ADR 0001)
so that tests and agents drive the shell without the widget tree. Several
choices made here are hard to change later: the crate boundaries, the GPUI
version, the text model, the threading model, and how UI commands, GPUI
actions and keymaps relate.

## Options considered

### GPUI version
1. crates.io `gpui = "=0.2.2"` (Zed commit 69e2130, October 2025): the
   version the toolkit prototypes measured; Apache-2.0 sub-crates are
   published with it; CI downloads crates only.
2. A git pin of a newer Zed commit: newer APIs and fixes, but CI fetches
   the whole Zed repository and the GPL application crates sit next to
   GPUI in the same checkout.
3. `gpui-pre` snapshots published for `gpui-component`: newer, but a third
   party's release cadence.

### Text buffer
1. `ropey` 1.x (MIT): a B-tree rope with char, byte, line and UTF-16
   indexing, cheap clones (shared nodes), widely used by editors.
2. `crop` (MIT): a byte-indexed rope, smaller API.
3. Our own rope on a sum tree, as Zed does (Zed's own crates are GPL and
   are not an option to copy).

### UI commands
1. Every UI command is a plain function on the workbench, called by a
   generic GPUI action `RunCommand { id, args }`.
2. Every UI command is its own GPUI action whose action name is the command
   id; components handle the actions they own; the registry lists the
   metadata and dispatches by id.

## Decision

### Crates
Platform crates (no language knowledge, ADR 0005):

| crate | role | GPUI |
|---|---|---|
| `text` | rope buffer (`ropey`), line index, selections (multi-caret ready), edit transactions, undo/redo with grouping, line separator and encoding detection and preservation | no |
| `syntax` | tree-sitter driver: incremental parse off the UI thread, highlight spans for line ranges, capture names resolved to `themes` text roles; grammars and queries come from the product | no |
| `project` | open a folder, lazy file tree model (ignore rules, hidden folders), async load and save of documents | no |
| `ui` | our component layer on GPUI: theme tokens as GPUI colours, fonts, metrics, icon loader (colour SVG rasterized with `resvg` at device scale), common components (icon button, island, tree rows, tabs, scrollbars, tooltips), the UI command index and invocation | yes |
| `editor` | the editor entity and element: virtualized line layout and painting, gutter, carets and selections, scrolling, mouse selection, clipboard, IME through GPUI's input handler | yes |
| `workbench` | the main window: toolbar, stripes, project tool window, tabs, status bar, keymaps, application start-up, bench mode | yes |

Product crates (`crates/python/`):

| crate | role |
|---|---|
| `py-syntax` | the Python grammar (`tree-sitter-python`) and highlight query, plus Python-specific refinements (`self`, dunder names, constants, built-ins) |
| `pyoxide-ide` | the `pyoxide-ide` GUI binary: composes the workbench with Python file types and highlighting |

Toolkit code stays in `ui`, `editor` and `workbench`; `text`, `syntax` and
`project` have no GPUI dependency and are tested without it.

### GPUI
crates.io `gpui = "=0.2.2"` (option 1). It has what the shell needs:
custom elements, `uniform_list`, the platform input handler (IME), key
contexts and keymaps, transparent title bar with positioned window
buttons, windows that open without focus, `TestAppContext`. A newer GPUI
is adopted deliberately when a missing feature or fix needs it; the `ui`
layer keeps the change contained. On macOS without Xcode the
`runtime_shaders` feature compiles the Metal shaders at start-up: it is a
default feature of the `pyoxide-ide` binary and is enabled for tests
through dev-dependencies; a release build with Xcode turns it off
(`--no-default-features`) to precompile them.

### Text model
`ropey` (option 1). Positions in the `text` API are char offsets with
conversions to bytes (tree-sitter), UTF-16 (the platform input handler) and
line/column. Documents hold text with `\n` line breaks; the detected
separator (LF, CRLF, CR) and the encoding (UTF-8 with or without BOM,
UTF-16 with BOM) are kept and restored on save, as the reference IDE does.
Edits are grouped into transactions that remember the selections before and
after; consecutive typing and deleting of the same kind merge into one undo
step until the caret moves, the kind of edit changes, or a pause of one
second passes.

### Threading
The UI thread owns all entities and renders. File system access,
directory listing, parsing and highlight queries run on GPUI's background
executor; results come back to the UI thread through the task that started
them and are applied only if the document version still matches. Dropping
a task cancels it. Nothing on the UI thread is proportional to the file
size: rendering, highlighting lookup and caret movement work on the visible
lines; opening a file reads, decodes and builds the rope in the background.
The syntax tree is told about every edit at once, so highlighting stays in
place while the incremental reparse runs.

Frames follow the display's refresh (GPUI's display link). After a
keystroke the main window draws at once instead (macOS: the layer's display
callback), so the typed text is ready for the next refresh rather than the
one after; at 60 Hz this takes keystroke-to-frame p99 from about 19.5 ms to
about 6 ms (replica/perf.md). Text typed into a terminal is drawn when its
echo is back from the shell instead (ADR 0014): one such frame is shown per
refresh, and a frame drawn before the echo would hold it back a refresh.

### Highlighting
Highlighting has two layers, as in the reference IDE, whose colours come
from a lexer and whose errors only add a mark. A language's line lexer
(`syntax::Lexer`, for Python `py_syntax::PythonLexer`) colours keywords,
literals, comments, decorators, operators, built-ins, the name after `def`
and `class` and calls, line by line with a small state between lines (an
open triple-quoted string); it needs no parse tree, so a file is coloured
before its first parse and incomplete code keeps its colours. The
tree-sitter captures are painted on top for what needs structure (methods,
docstrings, keyword arguments, f-string expressions), except in the
statements that contain an ERROR or MISSING node, where the tree's shape
is a guess; the error itself gets the theme's error underline. The lexer
states of all lines are computed with the background parse, so the UI
thread only lexes the lines it paints.

### Saving
There is no unsaved state to lose: a modified file is written when its tab
closes, when another tab is selected, when the window loses the focus to
another application, when the application quits, and after 2 seconds
without keyboard or mouse input
or edits (`workbench::SaveSettings`, defaults in code until there is a
settings window). The reference IDE saves on frame deactivation by default
and has an idle save that is off by default (15 s, 1 to 300 s); Pyoxide
turns the idle save on with a shorter time and adds the save on tab
switch. Idle time is a timer that is armed by input and, when it fires
early because of later input, sleeps for the rest of the time; there is no
polling. While a file is modified its tab shows a 6 px dot in the theme's
accent colour in place of the close button, which comes back on hover. The
reference IDE draws the same dot (in its information badge colour) only
when "Mark modified" is switched on, which is off by default; Pyoxide
always shows it.

### Reloading
Files changed outside the application are found as the reference IDE
finds them: the project folder is watched (`file-watcher` on the `notify`
crate in place of the reference's native `fsnotifier`), the open files are
looked at when the window gets the focus ("Synchronize files on frame
activation", on by default; `workbench::SyncSettings`), and Reload All
from Disk saves everything, then looks at everything. A file differs when
its size or modification time differ from the application's last read or
write and its text (or line separator, encoding, writability) differs
too; the application's own writes leave their stamp and a check that
overlaps one is dropped. An unmodified editor takes the new text as one
undo step of the part that differs (`text::ReloadPlan`, worked out off the
UI thread); carets inside it keep their column on the line a line diff
leads to, as the reference translates caret lines through a diff; the view
does not scroll. Undoing or redoing that step asks first ("Undo Reload
from Disk?"), as the reference does for this command.

An editor with changes of its own gets the reference's File Cache
Conflict dialog (Load File System Changes, the default; Keep Memory
Changes, also Escape; no Show Difference until there is a diff viewer).
No save writes over a newer file: every write first compares the file
with what the application last read or wrote (stamp, then content) and
leaves a file that changed meanwhile to the dialog, as the reference
refreshes a file before saving it and vetoes the save while a conflict
waits.

A file deleted outside the application closes its tab, as in the
reference; a rename outside is a deletion and a creation. A missing file
is looked at again after 300 ms first, since tools such as `git` remove a
file and write it again. One deviation: an editor with changes stays
open, its tab title in the error colour, and is not saved automatically,
because Pyoxide has no Local History yet to get the changes back from;
Save writes the file again, and closing the tab asks with the reference's
prompt for editors that are not saved automatically (its LightEdit
"Close Document": Save, Discard, Cancel). Open files are also written
when the application quits, as the reference saves all documents on exit.

Created and removed paths list the project tree's folders again, update
the file index and reach the language services as watched-file changes,
also for the application's own writes of files without an editor
(Replace in Files). When events were lost, or without a watcher, the tree
and the index are built again instead. The folder of an open file
outside the project folder, or of any open file without a project, is
watched too (without its subfolders), and the watcher starts with the
first such file: the reference watches only project and library roots,
but it always has a project, while Pyoxide opens single files, and a
window that never gets the focus would otherwise never see their
changes.

### Commands, actions and keymaps
Option 2. Every user-visible action of the shell is a UI command with a
stable dotted id (`project.open-folder`, `editor.undo`, `editor.copy`,
`tabs.close`, `workbench.toggle-project`, ...), declared once with its
title, summary and example arguments. The declaration creates a GPUI action
whose action name is the command id, so a keymap entry, a menu item, the
command palette, a test and the future control channel (#13) all reach the
same handler:

- Components handle their own actions through GPUI's dispatch (`on_action`
  on their element), so focus decides the target as with a key press.
- The workbench root also handles editor commands and forwards them to the
  active editor, so `editor.*` commands invoked by id work when the editor
  is not focused.
- `ui::command::invoke(id, args, window, cx)` builds the action from JSON
  arguments, checks that it is available, and dispatches it; read-only
  query commands (`workbench.state`) return JSON directly. Errors use the
  codes of ADR 0001 (`not_found`, `invalid_args`, `not_available`).
- The UI command index lists UI commands next to the engine commands of the
  `commands` registry; engine commands keep running on a background
  runtime.
- The control channel will receive `(id, args)` on its own thread and post
  them to the UI thread through a foreground task, which calls `invoke` and
  sends the result back. This is the session state ADR 0001 left open: the
  state lives in GPUI entities on the UI thread and commands that need it
  run there.

Keymaps are our own TOML files that bind keystrokes (GPUI syntax,
`cmd-shift-z`) to command ids with optional arguments, per key context
(`Editor`, `ProjectTree`, `GoToLine > Editor`, or global). Two default
keymaps ship, macOS and Windows/Linux, following the reference IDE's
default keymaps (keystrokes taken from the open-source IntelliJ Platform
keymap files, see NOTICE). A test presses every binding of both keymaps as
a real keystroke in a headless window and checks that it resolves to its
command in its context and has the command's effect; a binding without
such a check fails the test.

Language-specific editing is data of the product's file type, not code in
the editor: the block opener (`:`), block-ending words (`return`, ...),
the line comment prefix (`# `), paired quotes, triple quotes and string
prefixes. Extend Selection asks the file type's highlighter for the syntax
nodes around the selection, so a language with a tree-sitter grammar gets
syntax-aware selection without more code.

### Tests and measurements
- Headless tests run the real workbench in GPUI's `TestAppContext`. The
  test platform draws only when a test asks (`window.draw`), its windows
  start inactive (focus listeners need `activate_window`), and query
  commands are invoked through `update_window` on the window handle (the
  root view must not be leased while a query reads it).
- Keystrokes are tested as keystrokes (`simulate_keystrokes`), mouse input
  as simulated mouse events; behaviour that the reference IDE defines is
  written down first in `tests/scenarios/`.
- Measurements run inside the release binary (`PYOXIDE_BENCH`, see
  `crates/platform/workbench/src/bench.rs` and replica/perf.md): keys go
  through the window's key dispatch; frame start and end come from the
  workbench's own render and paint. The window stays above other windows
  without taking the focus while measuring, because a covered window gets
  no frames from the display.

### Build and run
The IDE runs with `cargo run --release -p pyoxide-ide [folder or files]`;
measurements and manual checks use release builds. Debug builds compile
the dependencies (GPUI, tree-sitter, ropey, ...) with `opt-level = 2`
(`[profile.dev.package."*"]` in the workspace manifest): GPUI's layout and
text shaping and the parser run far slower without optimization, and a
debug build should stay responsive enough for manual checks, while the
workspace crates keep fast incremental builds and full debug information.
The cost is a longer first build of the dependencies. The checks
(`scripts/check.sh`, which CI's `check` job calls) build with the same
profile: CI restores the compiled dependencies from its cache, so a pull
request builds only the workspace crates, and the GUI tests run three to
four times faster than with unoptimized dependencies (replica/perf.md, CI).
Until 2026-10-19 the checks built the dependencies without optimization
(`--config 'profile.dev.package."*".opt-level=0'`), which kept a cold build
short (the Windows job took 28-36 min with optimized dependencies and about
6 min without, when the test suite was small); with the suite grown, the
slower tests cost more than the shorter cold build saved.

### Look
The shell reproduces the reference IDE's current New UI ("Islands") layout
and metrics. Metrics live in the `themes` crate (new metrics are added
there, with docs/theme-mapping.md kept in sync), fonts are Inter (UI) and
JetBrains Mono (editor), both SIL OFL 1.1 and shipped with the binary, and
icons are the New UI icons of the IntelliJ Platform Community repository
(Apache-2.0, NOTICE).

## Consequences
- Eight new crates; GPUI adds about 350 crates to the build graph of the
  workspace, and Linux CI needs the X11, Wayland, xkbcommon and fontconfig
  development packages.
- On Linux `libc` is pinned to 0.2.189 (in the `ui` crate): GPUI 0.2.2
  depends on `xattr` 0.2.3, which uses `libc::ENOATTR`, removed in libc
  0.2.190. The pin goes away with a GPUI release that no longer pulls in
  that `xattr`.
- On macOS the main toolbar sits in a transparent title bar: dragging the
  toolbar moves the window and a double click zooms it (checked by hand;
  the test platform has no window server).
- Every new UI feature starts with its command declaration; widgets hold no
  logic beyond calling commands.
- `editor` and `workbench` tests run headless in `TestAppContext` (no
  window, no GPU); visual checks capture the app's own window on macOS.
- Upgrading GPUI touches `ui`, `editor` and `workbench` only.
- The text model normalises line breaks to `\n`; a file with mixed
  separators is saved with the first one found, which matches the
  reference IDE.
