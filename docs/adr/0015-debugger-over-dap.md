# 0015. Debugger over DAP with debugpy

- Status: accepted
- Date: 2026-10-16

## Context
A Python IDE needs a debugger: breakpoints in the gutter, stepping, frames,
a variables tree, watches, evaluation and a console with standard input
(#90). The reference's UI for this is the XDebugger UI of the IntelliJ
Platform (Debug tool window, Threads & Variables, Console, Breakpoints
dialog, Evaluate dialog, inline values). In 2026.2.3 the reference has two
Python backends behind the same UI: its own pydevd protocol and debugpy
over the Debug Adapter Protocol (DAP), the latter as the default, with a
"Mode" switch in the Debug tool window. It bundles debugpy 1.8.21.

Pyoxide follows the reference's UI and uses one backend. Constraints: DAP
is the public protocol for this; external processes only behind protocols;
the UI never waits for the adapter; platform crates know no language
(ADR 0005); the LSP client (ADR 0010) already has framing and a thread
model over the `process` crate.

Sources: IntelliJ Community (Apache-2.0),
https://github.com/JetBrains/intellij-community @ `pycharm/2026.2.3`:
platform/xdebugger-impl, platform/xdebugger-api, python/src/com/jetbrains/
python/debugger, python/helpers/debugpy (MIT); the DAP specification;
debugpy's PyPI metadata.

## Options considered
### Backend
1. debugpy over DAP (MIT, vendors pydevd under EPL-2.0). Public protocol,
   maintained, the reference's default.
2. The reference's pydevd protocol. Undocumented wire format; the
   reference's helpers are not used.

### Client
| option | licence | verdict |
|---|---|---|
| `dap` (dap-rs) 0.4.1-alpha1 | MIT OR Apache-2.0 | alpha, adapter side, last release 2023 |
| `dapts` 0.0.6 | MIT | generated types only, small user base; a source of types if the message set grows |
| `emmy_dap_types`, `debugserver-types`, `debug_types`, `dap-types` | MIT / MIT-Apache | stale or small |
| Zed's DAP crates | GPL-3.0 | never read |
| own client on the `lsp-client` model | - | **chosen** |

## Decision
debugpy over DAP, with Pyoxide's own client.

### Protocol and threads
- `stream-framing` (platform, new): the `Content-Length` framing moved out
  of `lsp-client` (it uses no protocol types); `lsp-client` and
  `dap-client` both use it.
- `dap-client` (platform, no toolkit): one adapter is one `process` child
  with piped standard streams; a reader thread parses frames and answers
  pending requests through oneshot channels (`seq` / `request_seq`), a
  writer sends frames; dropping a request's future cancels it (`cancel`
  where the adapter supports it). Reverse requests go to a handler.
  Hand-written serde structs for the ~25 messages used, private to the
  crate, `serde_json::Value` for rare fields. A transport trait: standard
  streams now, TCP later (attach, subprocesses). A fake adapter binary for
  tests, as `lsp-fake-server`.
- `debugger` (platform, no toolkit): sessions and their state machine,
  threads, frames, a variables tree with lazy, paged children,
  breakpoints (line, temporary, condition, log message, enabled, suspend
  policy, exception filters), watches per run configuration, mute, the
  product hook `DebugAdapterProvider`, storage and import, the `debug.*`
  engine commands (`debug.state`, `debug.step-over`, ...). UI crates
  depend on `debugger`, never on `dap-client` (`scripts/check-layers.py`).

### Launch (debugpy)
- Adapter: `python <helper>/debugpy/adapter`, `python` being the run's
  interpreter. `initialize` advertises `supportsRunInTerminalRequest`; then
  `launch` with the program or module, arguments, working directory and
  environment from the run configuration, `console: integratedTerminal`,
  `justMyCode: false` (the reference's Step Into enters libraries),
  `showReturnValue` from the settings, `subProcess: false`, a variable
  presentation that groups special variables and hides functions, classes
  and modules (the reference's Simplified Variables View).
- `runInTerminal` is answered by spawning the given command line with the
  `process` crate and feeding its output to the Debug Console exactly as
  the Run console (standard input, ANSI, links, Stop); the integrated
  terminal of DAP is this console, not the Terminal tool window.
  `internalConsole` would lose standard input.
- After `initialized`: `setBreakpoints` per file, `setExceptionBreakpoints`
  (`uncaught` by default, the reference's "Any exception, On termination"),
  `configurationDone`; the `launch` answer is not awaited before that.
- Step Into My Code: `setDebuggerProperty {justMyCode: true}` for one step.
  Run to Cursor: a temporary breakpoint and `continue`.
- Stop: `terminate`, then `disconnect {terminateDebuggee: true}`; after a
  timeout the debuggee's process tree and the adapter are killed. Rerun is
  stop and a new session. Adapter, launcher and debuggee must all be gone
  after Stop, closing the tab, closing the project or a crash, on every OS.
- One backend, so no Mode switch. Attach to Process, subprocess debugging
  (`startDebugging` or `debugpyAttach` over TCP) and remote targets come
  later.

### debugpy distribution
- Pinned to 1.8.21, the reference's version: it supports Python 3.8 and
  later (1.8.22 requires 3.10). An older interpreter gets an error naming
  the requirement.
- Shipped as a helper folder: the contents of the pure-Python wheel
  `debugpy-1.8.21-py2.py3-none-any.whl` (about 5.4 MB), unmodified, in the
  macOS bundle's `Resources/helpers/debugpy`, next to the binary on Windows
  and Linux. Release packaging fetches the wheel from PyPI with its pinned
  SHA-256; development builds run the same script once into the
  application cache. The folder is not committed. The application never
  downloads at run time without asking, and never takes debugpy from the
  project's environment (the launcher path is explicit).
- Licences: debugpy is MIT; its vendored pydevd is EPL-2.0; other vendored
  parts are listed in the wheel's `ThirdPartyNotices.txt`. Redistributing
  unmodified EPL-2.0 source requires: the EPL-2.0 text shipped with it,
  every copyright and licence notice in the files kept, a notice that those
  files are under EPL-2.0 with where their source is (the files are
  source; the notice also names the upstream repository and version), and
  no relicensing. Any compiled parts of the wheel shipped as such need the
  same statement of where their source is. Running pydevd as a separate
  process does not put Pyoxide's own code under EPL-2.0; a change to a
  pydevd file would be EPL-2.0 and published as source, so the files are
  not patched. NOTICE and a third-party notices file in the bundle carry
  this.

### Python product
`py-debug` (python, no toolkit): helper lookup, launch arguments from the
Python run configuration (`py-run`), the stoppable-line rule of the
reference (code lines, not blank, comment, docstring or continuation
lines), exception breakpoint mapping, inline value positions (the
parameter or first binding of a name in the frame's function), Special
Variables and Return Values groups, Python debugger settings and their
import.

### UI
`debugger-ui` (platform, GPUI), like `code-intel-ui`: the Debug tool window
(session tabs, toolbar, Threads & Variables with the evaluate field and
watches, Console), the Breakpoints dialog, the breakpoint properties popup,
inline values, value tooltips and the editor wiring through the editor
decorations API. Expressions are evaluated in the Variables pane; the
Evaluate dialog comes later. The Run console view
moves from `workbench` into a small `console-ui` crate so that Run and
Debug share it. The run widget, its dropdown and the Run/Debug
Configurations dialog get their Debug actions (drawn disabled today).

### Storage
Breakpoints, watches per configuration and the mute state in the personal
project state (ADR 0009, `[debugger]`): paths relative to the project root
with `/`, lines as shown (from 1). Project options in `[python.debugger]`
of the personal state, application options in the settings store
(ADR 0017). Imported once from `.idea/workspace.xml` (`XDebuggerManager`:
line breakpoints, default exception breakpoints, watches;
`PyDebuggerOptionsProvider`) with a marker in the personal `[debugger]`
table; `.idea` is never written.

## Consequences
- Four platform crates (`stream-framing`, `dap-client`, `debugger`,
  `debugger-ui`), a small `console-ui` move, one product crate.
- The application ships Python sources under MIT and EPL-2.0 in a helper
  folder; release packaging and NOTICE handle them. CI tests that need a
  real debugpy and Python are `#[ignore]`; the client is tested against
  the fake adapter on all three systems.
- `dap-client` and `debugger` run on threads and callbacks like
  `lsp-client`, with no async runtime: a request is a future completed by
  the reader thread, and Stop reports completion on a channel. A
  transport is a pair of traits (`Outlet`, `Inlet`): an adapter's standard
  streams, or memory. The feature `fake` of `dap-client` starts scripted
  adapters in a test's process (`fake::adapter`), the `dap-fake-adapter`
  binary starts them as a process; the framing, cancellation, ordering of
  `initialized` before the `launch` answer, `runInTerminal`, adapter
  crash and both shutdown paths are tested with them on all three
  systems. `debugger` re-exports the client's plain value types, so UI
  crates name only `debugger`; `scripts/check-layers.py` fails when a
  crate that uses the toolkit depends directly on `dap-client`.
- Three processes per session (adapter, launcher, debuggee); their
  lifetime is the riskiest part, especially on Windows.
- The editor gets gutter icons, line backgrounds and after-line texts
  (architecture.md, "Cross-cutting foundations"); there is no editor
  context menu yet, so Run to Cursor is reached by keys and the toolbar
  until it exists.
- Not in the first version: the Evaluate dialog and Quick Evaluate, Smart
  Step Into, Jump To Cursor, Reset Frame,
  the interactive debug console, logging breakpoint details, dependent
  breakpoints, attach, remote, data views, type renderers, the debugger
  settings pages.
