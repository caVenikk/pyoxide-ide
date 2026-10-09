# pyoxide-db

Command-line front-end of the Pyoxide database engine: Oracle (thin driver or
Oracle Client), PostgreSQL and SQLite. Every sub-command is also a `db.*`
command of the IDE (`pyoxide-db commands` lists them).

## Install
- From CI: the `binaries` job of the CI workflow uploads `pyoxide-db` for
  Windows (x86_64), macOS (Apple Silicon) and Linux (x86_64) as artifacts of
  each run on `main` (GitHub, Actions, the run, Artifacts).
- From source: `cargo build --release -p db-cli`, binary in
  `target/release/pyoxide-db` (`.exe` on Windows). Windows needs the MSVC
  build tools that `rustup` asks for.

## Connections
A connection is a saved profile (`--profile NAME`, ADR 0004) or inline flags:

```bash
pyoxide-db profile add reporting --backend oracle-thin --host db.example.com --service ORCLPDB1 --user app_reader
pyoxide-db profile add local-pg --backend postgres --host localhost --port 55432 --database pyoxide --user pyoxide
pyoxide-db profile add scratch --path ~/data/app.db --create
pyoxide-db profile list
pyoxide-db --backend sqlite --path :memory: query "select sqlite_version()"
```

Profiles live in `db-profiles.toml` in the user config directory
(`PYOXIDE_DB_PROFILES` overrides it) and never contain passwords. Oracle
options: `--sid` instead of `--service`, `--dsn` for a TNS alias or connect
descriptor (Oracle Client backend), `--tls require|verify-ca|verify-full`,
`--wallet-dir`, `--ca-file`, `--server-name`.

## Passwords
Looked up in this order: the profile's `password_env` variable,
`PYOXIDE_DB_PASSWORD`, the OS credential store, an interactive prompt.

```bash
pyoxide-db secret set reporting            # prompt without echo
pyoxide-db secret set reporting --stdin    # one line from stdin
pyoxide-db secret set reporting --wallet   # wallet password
pyoxide-db secret status reporting         # where it would come from
```

Passwords are never accepted as arguments and never printed.

## Everyday commands

```bash
pyoxide-db -p local-pg test
pyoxide-db -p local-pg query "select * from orders" --max-rows 100 --stats
pyoxide-db -p local-pg schemas
pyoxide-db -p local-pg objects --kind table --kind view
pyoxide-db -p local-pg describe public orders
pyoxide-db -p local-pg source active_orders --kind view
pyoxide-db --json -p local-pg query "select 1"
pyoxide-db call db.query '{"profile": "local-pg", "sql": "select 1"}'
```

## Diagnosing a connection

```bash
pyoxide-db -p reporting diagnose
pyoxide-db -p reporting diagnose --deep --try-all --share
```

One line per step: environment, profile files (CA, wallet, SQLite file),
password source, client availability, DNS, TCP, TLS, login, server version,
session, character set and time zone, dictionary access, a typed-value probe;
`--deep` adds paging and a 1 s statement timeout; `--try-all` also tries the
other Oracle backend; `--share` masks host names, addresses, user, service,
paths and session names so the report can be shared. The first failure shows
a cause and a next step. Exit code 1 when a step fails.

### On a Windows machine with an Oracle 19c server
1. Get `pyoxide-db.exe` (see Install) and open a terminal in its folder.
2. Create a profile with the same host, port, service name (or SID) and user
   as in the existing IDE data source:
   `pyoxide-db.exe profile add work --backend oracle-thin --host <host> --port 1521 --service <service> --user <user>`
3. Store the password: `pyoxide-db.exe secret set work`.
4. Run `pyoxide-db.exe -p work diagnose --deep --try-all --share` and keep the
   output.
5. If the thin backend fails with native network encryption, an
   authentication protocol error or a closed connection during login, install
   Oracle Instant Client (Basic or Basic Light, 64-bit), add its folder to
   `PATH`, and run the same command again: `--try-all` then also tries the
   `oracle-odpi` backend. A profile with `--backend oracle-odpi --dsn <alias>`
   uses `tnsnames.ora` from `TNS_ADMIN`.
