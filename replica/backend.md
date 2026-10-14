# Engine services checklist

Status of the engine services (the "backend" of the IDE) and the security
checklist from the skill pack, adapted to a desktop IDE.

## Services
| service | crate | status |
|---|---|---|
| Database drivers | `db-oracle-thin`, `db-oracle-odpi`, `db-postgres`, `db-sqlite` | M1, issues #1-#4 |
| Connection profiles, diagnostics, `db.*` commands | `db-service` | M1, #5 #6 |
| Secret stores (OS store, KeePass, memory) | `secrets` | M1, #5 #7 |
| Settings import from PyCharm | `settings-import` | #7 (data sources), #14 (themes) |
| LSP client (framing, sync, encodings, diagnostics, routing) | `lsp-client` over `process`, model `code-intel` | M3 (ADR 0010) |
| Python language services: ruff, ty | `py-lsp` | M3 (ADR 0010, 0011) |
| DAP client (debugpy) | - | M4 |
| Git | - | M5 |
| Docker | - | M7 |

## External tools
Detected and version-checked at runtime; the IDE degrades gracefully when one
is missing. Database: Oracle Instant Client is optional (only the
`oracle-odpi` backend needs it; `pyoxide-db backends` and `diagnose` report
it).

Python language services (M3):

| tool | lookup | version check | when missing |
|---|---|---|---|
| ruff (`ruff server`) | interpreter `bin`/`Scripts` -> custom path or PATH and per-user folders -> `uvx ruff` (Lookup sdk / path / uvx, as the reference) | `ruff --version` | not started; Language Services widget hidden or row Terminated with the error output |
| ty (`ty server`), default type server | same | `ty --version`, minimum version (ADR 0011) | not started; the editor works without code intelligence; notice in the widget |
| basedpyright (`basedpyright-langserver --stdio`), alternative type server | same | `basedpyright --version` | same |

Servers are stopped on project close and on quit; a crash leaves a
Terminated row with the server's standard error and a Restart action (no
automatic restart). Server memory is reported next to the application's.

## Security checklist
| item | state |
|---|---|
| Secrets only from the environment, the OS store, KeePass or a prompt; never arguments or files | done (ADR 0004) |
| `Secret` redacted in `Debug`, not serializable, zeroed on drop | done |
| `ConnectParams` Debug redacts passwords | done (test) |
| Error messages scrubbed of known secrets; TOML errors never quote source lines | done (tests) |
| Diagnostics: secrets always removed; `--share` masks identifying values | done (tests) |
| Values bound, never spliced, in metadata SQL | done in contract; per dialect in #1-#3 |
| Oracle descriptors built only from validated values | #4 |
| No telemetry; no network access the user did not trigger | done |
| Integration tests use throw-away containers and local-only default passwords | done |
| Imports never read real secrets without explicit consent; dry-run shows presence only | #7 |
| Parsers of untrusted input fuzzed (profiles TOML, Oracle connect strings, JDBC URLs, `.icls`/`.theme.json`) | todo |
| Plugins sandboxed (WASM) | M7 |
| LSP frames from servers parsed defensively (length limits, invalid UTF-8, malformed JSON); framing fuzzed | M3 |
| Workspace edits from servers applied only to files of the project or open documents, as one undo step | M3 |
| `uvx` fallback downloads a tool on first use: asked once for Pyoxide's default tools, silent for tools the user enabled, as the reference (ADR 0010) | M3 |
