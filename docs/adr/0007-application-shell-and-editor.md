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
about 6 ms (replica/perf.md).

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
Headless tests run faster too. The cost is a longer first build of the
dependencies.

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
