# 0003. Database drivers for the MVP

- Status: accepted
- Date: 2026-10-09

## Context
The MVP covers Oracle (19c and later), PostgreSQL and SQLite. Application code
is Rust; non-Rust dependencies need a recorded decision.

## Options considered

| Engine | Option | Notes |
|---|---|---|
| Oracle | `oracle-rs` (pure Rust thin protocol, MIT/Apache-2.0) | No client install; TLS and `ewallet.pem` wallets; 11g/12c password verifiers. No public statement cancel. No native network encryption, Kerberos or LDAP naming. Young crate. |
| Oracle | `oracle` crate over ODPI-C (UPL/Apache-2.0) | Needs an installed Oracle Client at runtime (loaded dynamically, so the build does not). Full feature set: `break_execution`, call timeouts, TNS aliases, wallets, NNE, Kerberos. |
| PostgreSQL | `tokio-postgres` (MIT/Apache-2.0) | Pure Rust, async, cancel tokens, binary protocol. |
| PostgreSQL | `sqlx` | Heavier, compile-time query focus, weaker cancel story. |
| SQLite | `rusqlite` with the `bundled` feature (MIT; SQLite is public domain) | Compiles the SQLite C amalgamation; the de-facto standard. |
| SQLite | Pure-Rust SQLite rewrites | Not yet file-format and feature complete. |
| TLS | `rustls` with the `ring` provider (Apache-2.0/ISC/MIT) | Pure Rust API; `ring` builds with a C compiler only, no CMake/NASM on Windows. OS trust store via `rustls-native-certs`. |

## Decision
- Oracle: `db-oracle-thin` (`oracle-rs`) is the default; `db-oracle-odpi` is
  the fallback for environments thin cannot reach. `pyoxide-db diagnose`
  detects the typical thin limitations and points to the fallback.
- PostgreSQL: `db-postgres` on `tokio-postgres` with `rustls`.
- SQLite: `db-sqlite` on `rusqlite` with bundled SQLite. This is an accepted
  non-Rust dependency: a compiled-in C library with no external process and
  no runtime installation. ODPI-C in `db-oracle-odpi` is accepted on the same
  terms (it loads the Oracle Client only at runtime).
- TLS: `rustls` + `ring` everywhere it is configurable.

## Consequences
- Thin cancel: until `oracle-rs` exposes a break/reset, a timeout or cancel
  drops the call and the connection is marked broken and reopened. The ODPI
  backend cancels properly. Adding cancel upstream is a candidate contribution.
- Windows builds need a C compiler (MSVC) for `ring`, SQLite and ODPI-C; the
  standard Rust toolchain setup already requires it.
- Other engines (MySQL, SQL Server, ...) come after the MVP behind the same
  traits.
