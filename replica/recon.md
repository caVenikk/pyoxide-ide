# Recon map: PyCharm (desktop)

One section per feature area. Sources are public documentation and observed
behaviour only (see `docs/clean-room.md`). Reference screenshots stay in the
git-ignored `replica/screens/`.

---

## Area: db (database client engine)

Scope: the engine behind the database tools: connections, statement
execution, result paging, cancel and timeouts, typed values, schema metadata,
connection settings and secrets. The GUI on top (explorer tree, console, data
grid) is area `db-gui`.
Date: 2026-10-09

### Sources

| # | source | URL | notes |
|---|---|---|---|
| 1 | Data source settings reference | https://www.jetbrains.com/help/pycharm/data-sources-and-drivers-dialog.html | General, Options, SSH/SSL, Schemas, Advanced tabs |
| 2 | Configure connection | https://www.jetbrains.com/help/pycharm/configuring-database-connections.html | test connection, save password modes |
| 3 | Run queries | https://www.jetbrains.com/help/pycharm/run-a-query.html | execute, cancel running statements |
| 4 | Rows / paging | https://www.jetbrains.com/help/pycharm/rows.html | page size setting, default 500 |
| 5 | Sessions | https://www.jetbrains.com/help/pycharm/managing-connection-sessions.html | transaction mode, cancel per session |
| 6 | Query results | https://www.jetbrains.com/help/pycharm/viewing-query-results.html | result tabs, in-editor results |
| 7 | Oracle Database Reference (data dictionary) | https://docs.oracle.com/en/database/oracle/oracle-database/19/refrn/ | ALL_* views for introspection |
| 8 | Oracle Database Error Messages | https://docs.oracle.com/en/database/oracle/oracle-database/19/errmg/ | ORA-/TNS- codes for diagnostics |
| 9 | PostgreSQL system catalogs | https://www.postgresql.org/docs/current/catalogs.html | pg_catalog, information_schema |
| 10 | SQLite schema table and PRAGMAs | https://www.sqlite.org/schematab.html, https://www.sqlite.org/pragma.html | sqlite_schema, table_info, index_list |

### Core loop
Configure a data source once, test it, then run SQL and browse results and
schema objects without leaving the IDE.

### Screens (engine-facing; the GUI is area db-gui)

| ID | screen | how to reach | purpose | key components | states |
|---|---|---|---|---|---|
| S-DB01 | Data source settings | Database tool window, `+`, data source | connection parameters | host, port, database/service, user, password + save mode, URL, driver, Test Connection | empty, filled, testing, success (server + driver version), error |
| S-DB02 | Options tab | S-DB01 | session behaviour | read-only, transaction control Auto/Manual, keep-alive, auto-disconnect, startup script, session control | default, changed |
| S-DB03 | SSH/SSL tab | S-DB01 | network security | SSH tunnel, SSL: CA file, client cert/key, truststore | off, on |
| S-DB04 | Schemas tab | S-DB01 | what to introspect | schema pattern, object filter | all, filtered |
| S-DB05 | Query console | data source, New, Query Console | run SQL | editor, execute, cancel, results | idle, running, result, error, cancelled |
| S-DB06 | Result grid | S-DB05 or open a table | view rows | page size (default 500), paging, NULL display, typed cells | empty, page, last page, error |

### Flows

```
F-DB01 Connect to a database
    S-DB01 -> Test Connection -> success shows server and driver versions
    edge: wrong password, unknown service/database, host unreachable,
          TLS required, missing client library (Oracle thick), timeout

F-DB02 Run a query and page through results
    S-DB05 -> execute -> S-DB06 first page -> next pages
    edge: zero rows, millions of rows, very wide values (LOBs), NULLs,
          exact decimals, time zones, binary data, a DML statement

F-DB03 Cancel a long statement
    S-DB05 -> execute slow statement -> Cancel Running Statements
    edge: cancel during fetch, cancel when nothing runs, server ignores cancel

F-DB04 Browse schema metadata
    explorer -> schema -> tables/views/sequences/routines -> columns, keys,
    indexes, constraints, source
    edge: no privileges on dictionary views, thousands of objects,
          quoted mixed-case names

F-DB05 Manage connections and passwords
    S-DB01 -> save password (never / session / forever) -> reconnect later
```

### Inferred data model

```
DataSource   name, driver/backend, host, port, database|service|sid|url, user,
             password save mode, ssl {ca, cert, key, mode}, ssh, read_only,
             tx_mode, keep_alive, auto_disconnect, startup_script,
             schema filter, connection properties
             evidence: sources 1, 2, 5    confidence: high
Session      data source, tx mode, running statements
             evidence: source 5           confidence: high
ResultPage   columns (name, type), rows, page size, has_more
             evidence: sources 4, 6        confidence: high
```

Pyoxide mapping: `DataSource` = connection profile (TOML, ADR 0004);
`Session` = `db_core::Connection`; `ResultPage` = `Cursor::next_page`.

### Feature matrix
See `replica/features.csv`, areas `database` and `db-gui`. Database tools
are a PyCharm Professional feature; they are promoted to MVP priorities here.

### Out of scope for the MVP
- JDBC drivers and "any driver" support; engines beyond Oracle, PostgreSQL,
  SQLite.
- SSH tunnels (planned after the MVP).
- Kerberos/LDAP for the thin Oracle driver (available through the Oracle
  Client backend).

### Size
Engine: 4 drivers, 1 contract, profiles, diagnostics. Hard parts: typed
decoding per backend, cancel semantics per protocol, Oracle environments that
the thin protocol cannot reach. Size: L (about 30 to 40 agent-hours).
