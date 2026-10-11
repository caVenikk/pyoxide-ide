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
| LSP client, ruff, type checker | - | M3 |
| DAP client (debugpy) | - | M4 |
| Git | - | M5 |
| Docker | - | M7 |

## External tools
Detected and version-checked at runtime; the IDE degrades gracefully when one
is missing. Database: Oracle Instant Client is optional (only the
`oracle-odpi` backend needs it; `pyoxide-db backends` and `diagnose` report
it).

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
