# db-oracle-odpi

Oracle backend that talks to the database through an installed **Oracle
Client** (ODPI-C, via the [`oracle`](https://crates.io/crates/oracle) crate).
The client libraries are loaded at run time, so the crate builds and its unit
tests run without them.

## When to use it

`db-oracle-thin` (pure Rust, no installation) is the default. Use this backend
when the thin driver cannot reach the server or the setup needs something that
only the Oracle Client implements:

| Need | Thin | This backend |
|---|---|---|
| Native network encryption and integrity (`sqlnet.ora`) | no | yes |
| Kerberos, Windows (NTS) and other OS authentication | no | yes |
| TNS aliases (`tnsnames.ora`), LDAP/OID naming, `sqlnet.ora` rules | no | yes |
| Auto-login wallets (`cwallet.sso`), secure external password store | PEM wallets only | yes |
| Cancel a running statement and call timeouts | no | yes |
| Nothing to install | yes | no |

`pyoxide-db diagnose` points to this backend when it recognises one of the
thin driver's limits.

## Installing the Oracle Client

Install **Oracle Instant Client**, package "Basic" or "Basic Light", from
Oracle's download page. It must match the application's architecture (64-bit)
and may be any release from 11.2 to the current one; 19c or later is
recommended. Client call timeouts need 18c or later, and wallet directories in
the connect descriptor need 19c or later.

**Windows**
1. Install the Visual C++ redistributable the client release needs (Instant
   Client 19 needs the 2017 one, 21 and later the 2019 or newer).
2. Unzip the client, for example to `C:\oracle\instantclient_19_21`.
3. Make it findable by one of: adding the folder to `PATH` (before any other
   Oracle folder), copying the DLLs next to the application's executable, or
   setting the `oracle.lib_dir` option (below).

**macOS**
1. Unzip the client (or mount the DMG and run its `install_ic.sh`), then move
   it out of `~/Downloads`, which applications may not read.
2. Set `DYLD_LIBRARY_PATH` to the folder, put the libraries next to the
   executable, or set `oracle.lib_dir`. `libclntsh.dylib` must be in it.

**Linux**
1. Unzip the client (or install the RPM) and install `libaio` (`libaio1` on
   some distributions) and, for Instant Client 19 on recent distributions,
   `libnsl`.
2. Add the folder to the loader path (`ldconfig`, or `LD_LIBRARY_PATH`), or
   set `oracle.lib_dir`. `libclntsh.so` must be in it.

Without a usable client, `availability()` and `connect` fail with
`ErrorKind::Unavailable` and the `DPI-1047` message that lists where the
libraries were searched.

## Options

Set in `ConnectParams::options` (profile keys with the same names):

| Option | Meaning |
|---|---|
| `oracle.dsn` | Connect string used as it is: a TNS alias, Easy Connect (Plus), or a full `(DESCRIPTION=...)`. Host, port and database are then ignored. |
| `oracle.sid` | `database` is a SID, not a service name (`true`). |
| `oracle.lib_dir` | Folder with the Oracle Client libraries (Instant Client). |
| `oracle.config_dir` | Folder with `tnsnames.ora`, `sqlnet.ora` and wallets: the same as `TNS_ADMIN`, which also works. |

`oracle.lib_dir` and `oracle.config_dir` are read once per process, by the
first connection (or availability check) that loads the client; later values
are ignored. `init_client(lib_dir, config_dir)` does the same at start-up.

## Connect strings

Without `oracle.dsn` the connect string is built from the parameters, with
every value checked (host names, services and SIDs reject anything that could
change the structure of a descriptor) and the password never placed in it:

- plain service name, no timeout: `//host:1521/service`
- otherwise a descriptor, which every client from 12.2 accepts:
  `(DESCRIPTION=(CONNECT_TIMEOUT=10)(TRANSPORT_CONNECT_TIMEOUT=10)(ADDRESS=(PROTOCOL=tcp)(HOST=...)(PORT=...))(CONNECT_DATA=(SERVICE_NAME=...)))`
  with `(SID=...)` for SIDs, `tcps` and a `(SECURITY=...)` section for TLS.
  A 11.2 or 12.1 client needs `oracle.dsn` instead.

## TLS and wallets

- `tls.mode` other than `disable` selects `tcps`. The Oracle Client always
  validates the server certificate chain against its wallet; `require` and
  `verify-ca` only turn the distinguished-name check off, `verify-full` turns
  it on (`SSL_SERVER_DN_MATCH=yes`).
- `tls.server_name` for `verify-full` must be the certificate DN
  (`CN=db.example.com,O=...`), which becomes `SSL_SERVER_CERT_DN`. Without it
  the client compares the host name.
- `tls.wallet_dir` becomes `MY_WALLET_DIRECTORY` in the descriptor (documented
  for 19c and later clients; for older ones put `WALLET_LOCATION` in
  `sqlnet.ora` and use `oracle.config_dir`). The Easy Connect Plus form
  `?wallet_location=` is the same setting for 19c and later but is not
  generated here, because descriptors also work with older clients. The
  folder must hold an **auto-login wallet** (`cwallet.sso`). Oracle Cloud
  wallet downloads contain one next to `ewallet.pem`. A wallet password
  (`tls.wallet_password`) and the PEM options (`ca_file`, `client_cert`,
  `client_key`) are not usable by the client: the password is ignored, and PEM
  files without a wallet are rejected with an explanation.
- A wallet directory over plain `tcp` is passed through too, for a secure
  external password store.

## Kerberos and OS authentication

Leave `user` and `password` empty: the client authenticates with the operating
system or Kerberos identity (`external_auth`). It must be configured the usual
way: `SQLNET.AUTHENTICATION_SERVICES` (`KERBEROS5`, `NTS`, ...) and the
Kerberos settings in `sqlnet.ora`, found through `oracle.config_dir` or
`TNS_ADMIN`, and a valid ticket (`kinit` on Linux and macOS; the logged-in
session on Windows). With `user` set and an empty `password` the login is an
ordinary one and fails with `ORA-01005`.

## Behaviour

- **Threading.** Every connection owns one thread that makes all client
  calls (`db_core::worker`, shared with the SQLite backend); async callers
  wait on a channel. Cancel breaks the running call from
  another thread (`ORA-01013` becomes `ErrorKind::Cancelled`), and does
  nothing when no statement is running. A future dropped while a call is in
  flight breaks the call and marks the connection broken
  (`ErrorKind::ConnectionLost`).
- **Timeouts.** `ExecOptions::timeout` is set as the client's call timeout
  for each round trip (`DPI-1067` becomes `ErrorKind::Timeout`) and backed by
  a timer that cancels the statement; clients older than 18c use the timer
  alone.
- **Paging.** A query keeps its statement open in the connection thread and
  fetches `fetch_size` rows per round trip, only when `next_page` asks. The
  first page arrives with the execute round trip. Closing or dropping a cursor
  closes the statement without reading the rest.
- **Types.** `NUMBER` with scale 0 that fits `i64` is `Int`, other numbers are
  exact `Decimal` text in plain notation (padded to the column scale);
  `BINARY_FLOAT` and `BINARY_DOUBLE` are `Float`; `DATE` and `TIMESTAMP` are
  `Timestamp`; `TIMESTAMP WITH [LOCAL] TIME ZONE` are `TimestampTz` (a local
  time zone value carries the session's offset); intervals are text
  (`+01-02`, `+03 04:05:06.000000`); `RAW`, `LONG RAW` and `BLOB` are `Bytes`;
  text types, `LONG`, `CLOB` and `NCLOB` are `Text`; `ROWID`, objects, nested
  cursors and `BFILE` are `Other` (a `BFILE`'s file is not read). Values longer
  than `max_value_bytes` are `Truncated` with the full length (for a `CLOB`
  longer than four times the limit in characters, a short sample and the
  length in UTF-16 units).
- **Parameters.** Positional `:1`..`:n`. Text goes as `VARCHAR2` (so indexes on
  `VARCHAR2` columns stay usable), decimals as exact `NUMBER`, whole-second
  timestamps as `DATE`, `Bool` as 1 and 0. `Time` and `Array` are rejected.
- **Statements.** A trailing `;` is removed from SQL and kept for PL/SQL
  blocks and `CREATE` of stored units; a final `/` line is dropped.
- **Transactions.** `TxMode::Auto` commits after each DML or PL/SQL statement
  with one extra round trip; DDL commits by itself and queries need none.
  Switching from manual to auto commits the open transaction.
- **Read-only.** Statements other than queries are rejected before they are
  sent. In manual mode the session also runs inside `SET TRANSACTION READ
  ONLY`, restarted after every commit and rollback. In auto mode Oracle has no
  session-wide read-only setting, so the client-side check is the protection;
  PL/SQL blocks are rejected because they may write.
- **Not supported.** Native `JSON` and `VECTOR` columns (the `oracle` crate
  cannot fetch them; select `JSON_SERIALIZE(column)` instead), `OUT` and
  `RETURNING` binds, array binds, implicit results.

## Tests

```bash
cargo test -p db-oracle-odpi                  # unit tests, no client needed
docker compose up -d oracle                   # Oracle Free, FREEPDB1
cargo test -p db-oracle-odpi -- --ignored     # needs an Oracle Client
cargo test -p db-oracle-odpi --test it oracle::bench_fetch_100k -- --ignored --nocapture
```

See `docs/testing-databases.md` for the connection defaults and overrides.
