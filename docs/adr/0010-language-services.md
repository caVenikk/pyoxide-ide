# 0010. Language services over LSP

- Status: accepted
- Date: 2026-10-14

## Context
The reference IDE highlights problems, completes code, shows documentation
and parameter info, navigates to declarations and usages, applies quick
fixes and reformats code. For Python most of this comes from its own
engine; external tools (ruff, ty, Pyright, Basedpyright, Pyrefly) run as
language servers next to it, are off by default, and add diagnostics,
quick fixes, formatting and some completion and documentation results
(`replica/recon.md`, area python-intel). Pyoxide has no such engine. It
keeps the reference's editor behaviour and takes the data from language
servers: ruff and one type server (ADR 0011).

Constraints: platform crates know no language and never depend on product
crates (ADR 0005); UI crates depend on traits only; the UI thread never
waits for a server; the completion popup appears within 100 ms after the
server answers (`docs/performance.md`); external processes run only
behind protocols (AGENTS.md); Windows, macOS and Linux.

The reference's LSP client is open source at `pycharm/2026.2.3`
(`platform/lsp`, `platform/lsp-impl`, Apache-2.0) and is read for
behaviour: document sync, diagnostics mapping, completion rendering, the
Language Services widget. Zed's `lsp`, `editor` and `markdown` crates are
GPL-3.0 and are not read or copied.

## Options considered
1. A crate from crates.io for the protocol plumbing: `async-lsp` 0.2
   (MIT/Apache-2.0, tower, wants async pipes), `tower-lsp` and
   `lsp-server` (server side only). None fits the thread-based `process`
   crate (#45) without an adapter, and the framing is small.
2. Our own JSON-RPC client over `process` (reader and writer threads,
   `flume`, `futures` oneshot channels), protocol types from `lsp-types`
   kept private to one crate.
3. Language features inside the editor crate. Couples the editor to LSP
   and to one data source.

## Decision
Option 2, split into a model, a client, a UI layer and the product.

### Crates
| crate | layer | GPUI | role |
|---|---|---|---|
| `code-intel` | platform | no | the language-agnostic model and the `CodeIntel` trait: documents, positions as `text::Point`, diagnostics, completion items, documentation as Markdown, signatures, locations, text and workspace edits, code actions with sub-actions, server status and events. The `db-core` of code intelligence. |
| `lsp-client` | platform | no | the LSP client: framing, requests with cancellation, server-to-client requests, initialize / shutdown, position encodings, document sync, pull and push diagnostics, and `LspCodeIntel`, an implementation of `CodeIntel` over several servers with routing per feature. Protocol types (`lsp-types` 0.95, MIT) never leave the crate. |
| `code-intel-ui` | platform | yes | per-window controller attached to tab editors, the completion popup, quick documentation and hover, parameter info, declaration navigation and the usages popup, the intention actions popup, reformat and optimize imports commands, the Language Services status widget and popup, the `code.*` commands and the product hook. |
| `ui` (module `markdown`) | platform | yes | Markdown to GPUI elements (`pulldown-cmark`, MIT), code blocks coloured by a lexer the caller passes. |
| `editor` (extended) | platform | yes | the diagnostics of an editor (moved through edits), their painting, the error stripe and inspection widget, next and previous problem, hover and modifier events, a link highlight, geometry queries for popups. |
| `py-lsp` | python | no | Python language services: server specs for ruff (`ruff server`), ty (`ty server`) and basedpyright (`basedpyright-langserver --stdio`), executable lookup, settings, initialization options and `workspace/configuration` answers, ruff rule names, the one-time import of the reference's tool settings. |
| `pyoxide-ide` (extended) | python | yes | the product's `CodeIntelProvider`, the type engine widget, ruff's extra intentions (disable a rule for a file or the project), interpreter changes forwarded to the services. |

Dependencies: `lsp-client` -> `code-intel`, `process`, `text`;
`code-intel-ui` -> `code-intel`, `editor`, `ui`, `name-match`;
`workbench` -> `code-intel-ui`; `py-lsp` -> `lsp-client`, `code-intel`,
`project`, `py-env`. The editor does not depend on `code-intel`: it has
its own diagnostic marks. Checked by `scripts/check-layers.py`.

### Threads and cancellation
- One server = one `process` child with piped standard streams. A reader
  thread parses `Content-Length` frames and JSON, answers pending requests
  through oneshot channels and forwards notifications and server requests;
  the writer is `process::write_stdin` fed with serialised frames. No async
  runtime; GPUI tasks await the oneshot futures.
- A feature call returns a future; dropping it (a GPUI task dropped by a
  newer request) sends `$/cancelRequest`. Large answers are parsed on the
  reader thread; completion lists are filtered on the background executor.
- Shutdown: `shutdown` request, `exit`, kill after a timeout; on project
  close and application quit. Servers also exit on standard input EOF.
- No automatic restart after a crash, as in the reference: the server
  shows as Terminated with its error output and a Restart action.

### Documents and positions
- Only editors of tabs whose file type has a language id are documents;
  previews, consoles and one-line fields are not.
- `text::AppliedEdit` carries the inserted text and the UTF-16 columns of
  the replaced range, so every edit becomes one incremental `didChange`
  change, sent at once, without debounce (as the reference does). Each
  document has its own LSP version counter.
- The client offers the encodings `utf-8` and `utf-16` and converts per
  server; the model uses char-based `text::Point` in the text version the
  request was made for. Answers for an older version are dropped, or
  mapped through the edits since (diagnostics).
- `didSave` only to servers that ask for it; `didClose` when the tab
  closes. Changes the application makes on disk (save, create, delete,
  rename) are sent as `workspace/didChangeWatchedFiles` to servers that
  registered matching watchers. Changes made outside the application come
  the same way from the project's file watcher (`file-watcher`); an open
  file's change is sent once its check shows it is not the application's
  own write, and a reloaded editor sends its new text.

### Diagnostics
- Pull (`textDocument/diagnostic`, also when registered dynamically)
  after the reference's 300 ms highlighting delay, and push
  (`publishDiagnostics`).
- Severity mapping as the reference's `LspDiagnosticsCustomizer`: Error
  -> error, Warning -> warning, Information -> weak warning, Hint ->
  information (not counted); tag Unnecessary -> greyed like unused code,
  Deprecated -> struck out. Message shown as `<first line>  <source>(<code>)`.
- Two servers on one file: ruff gets `showSyntaxErrors = false` while a
  type server runs; marks with the same range from several servers become
  one mark listing all messages; code actions are merged, quick fixes
  first.

### Routing
Each server spec lists the features it serves. Python: ruff serves
diagnostics, code actions, formatting and organize imports; the type
server serves everything else (completion, hover, signature help,
definition, references, rename, symbols, highlights) and diagnostics and
quick fixes of its own. Formatting goes to one server only.

### Product hook
`workbench::Product` gets `code_intel: Option<CodeIntelHook>`; the
product's `CodeIntelProvider` builds the services of a project. Through
the `lsp_client::ServerCustomizer` of its server specs it answers
`workspace/configuration`, renames diagnostics and code actions, nests
them (ruff: rule names, Disable for this line as a sub-action) and adds
its own intentions. `FileType` gets a language id (`"python"`).

### Settings
Shared settings in `.oxide/project.toml`, personal executable paths in
`.oxide/workspace.toml` (ADR 0008, 0009):

```toml
[python.tools.ruff]
enabled = true
lookup = "sdk"          # sdk | path | uvx, as the reference's Lookup column
inspections = true
formatting = true
import-optimizer = true

[python.type-engine]
name = "ty"             # ty | basedpyright (ADR 0011)
```

```toml
# .oxide/workspace.toml
[python.tools.ruff]
path = "/opt/tools/ruff"   # optional custom executable
```

Executable lookup follows the reference: `sdk` tries the folder of the
project interpreter (`bin`, `Scripts` on Windows), then a custom path or
PATH and per-user tool folders, then `uvx`; `path` skips the interpreter;
`uvx` always runs `uvx <tool>`. Tools that are on only because they are
Pyoxide's defaults (ruff, the type server) do not download anything
without asking: when lookup would end at `uvx`, the Language Services
widget offers to run the tool through uv once and remembers the answer in
the personal state. Tools enabled in the settings or imported from the
reference behave as there (`uvx` without asking). The reference's settings are imported once
from `.idea/pyLspTools.xml` (`PyToolsState`, and the older form inside
`RuffConfiguration` and its peers) and `.idea/pyTypeEngine.xml` while
`project.toml` has no `[python.tools]` table; `.idea` is never written.
A Settings page comes with the settings window.

### Commands
`code.complete`, `code.quick-doc`, `code.parameter-info`,
`code.go-to-declaration`, `code.show-usages`, `code.show-intentions`,
`code.reformat`, `code.optimize-imports`, `code.next-problem`,
`code.previous-problem`, `code.error-description`, popup-internal
`completion.*`; queries `code.state`, `code.servers`; management
`code.restart-servers`, `code.stop-server`. Default keys come from the
reference's keymaps (Community, Apache-2.0).

## Consequences
- The editor UX can be built and tested against an in-memory fake
  `CodeIntel`, the client against a fake server binary, in parallel.
- `text::AppliedEdit` stops being `Copy` and grows by one `Arc<str>` per
  edit.
- Servers run as separate processes; their memory is reported next to the
  application's, not inside the idle budget of the application process.
- Not in the first version: Find Usages tool window, rename, highlight
  usages, Problems tool window, Go to Class / Symbol, File Structure,
  inlay hints, semantic tokens, external file changes, the settings page.
  The routing and the model leave room for them.
