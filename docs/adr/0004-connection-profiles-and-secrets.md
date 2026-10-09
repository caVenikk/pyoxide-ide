# 0004. Connection profiles and secrets

- Status: accepted
- Date: 2026-10-09

## Context
Users keep several database connections. Settings must be easy to edit, back
up and share; passwords must never be written to files, logs or command
output.

## Decision
**Profiles** are TOML, one table per profile, no secrets:

```toml
[profiles.reporting]
backend = "oracle-thin"        # oracle-thin | oracle-odpi | postgres | sqlite
host = "db.example.com"
port = 1521
service = "ORCLPDB1"           # Oracle; or: sid = "ORCL"
user = "app_reader"
connect_timeout_secs = 10
query_timeout_secs = 60
read_only = true
[profiles.reporting.tls]
mode = "verify-full"           # disable | prefer | require | verify-ca | verify-full
wallet_dir = "/path/to/wallet"
```

PostgreSQL uses `database`; SQLite uses `path`; the Oracle Client backend also
accepts `dsn` (TNS alias or full descriptor). Unknown keys are rejected.

Location: `PYOXIDE_DB_PROFILES` if set, otherwise
`<config dir>/pyoxide/db-profiles.toml` (`~/Library/Application Support` on
macOS, `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` or `~/.config` on Linux).

**Secrets** are looked up in this order:
1. The environment variable named by the profile's `password_env`, else
   `PYOXIDE_DB_PASSWORD`.
2. The OS credential store through the `keyring` crate (macOS Keychain,
   Windows Credential Manager, Secret Service on Linux), service
   `pyoxide-db`, account = profile name; wallet passwords use
   `<profile>/wallet`.
3. An interactive prompt without echo (CLI only, when stdin is a terminal).

Passwords are never accepted as command-line arguments. `pyoxide-db secret set
<profile>` reads the password from a prompt or stdin and stores it in the OS
store.

## Consequences
- Profiles can be committed or shared; secrets stay per machine.
- Headless Linux without a Secret Service falls back to environment
  variables; diagnostics report which source was used, never the value.
- The IDE settings UI edits the same file through `db.profile.*` commands.
