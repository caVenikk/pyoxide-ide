# Test plan

Generated from the flows in `replica/recon.md`. Case ids: `F-<flow>-H` happy
path, `-E` edge case, `-N` negative case. "Where" names the test layer:
unit (always runs), int (`#[ignore]`, Docker databases), cli (end-to-end
through `pyoxide-db`), scenario (`tests/scenarios/`).

## Area: db

### F-DB01 Connect to a database
| id | case | where |
|---|---|---|
| F-DB01-H1 | connect to Oracle, PostgreSQL, SQLite; server version shown | int per driver, cli `test` |
| F-DB01-E1 | Oracle by SID instead of service name | int thin |
| F-DB01-E2 | Oracle connect string (Easy Connect, descriptor, TNS alias) | unit `oracle_net`, int thin |
| F-DB01-E3 | TLS modes (require, verify-ca, verify-full) and wallets | unit TLS mapping, diagnose `tls` step |
| F-DB01-E4 | password from env, profile env, OS store, prompt | unit `secrets` |
| F-DB01-E5 | SQLite file missing with and without `create` | unit sqlite |
| F-DB01-N1 | wrong password (ORA-01017, 28P01) | int per driver, diagnose hint |
| F-DB01-N2 | unknown service or database (ORA-12514, 3D000) | int per driver |
| F-DB01-N3 | closed port, unresolvable host | unit diagnose, int |
| F-DB01-N4 | Oracle Client missing (DPI-1047) | unit odpi `availability` |
| F-DB01-N5 | password never in output, errors, profile files | unit `profiles`, `diagnose` (share), `params` Debug |

### F-DB02 Run a query and page through results
| id | case | where |
|---|---|---|
| F-DB02-H1 | first page of 500 without reading the rest; next pages continue | int per driver, sqlite unit |
| F-DB02-E1 | zero rows; DML returns affected rows | int per driver |
| F-DB02-E2 | 100,000 rows with continuity | int per driver, `db-bench` |
| F-DB02-E3 | exact decimals (38 digits), NULL vs empty string, Unicode | int typed round trip |
| F-DB02-E4 | dates, timestamps with fractions and offsets, intervals | int typed round trip |
| F-DB02-E5 | binary, LOBs over 1 MiB truncated with full length | int per driver |
| F-DB02-E6 | close a big cursor early, next statement runs at once | int per driver |
| F-DB02-N1 | statement while a cursor is open: `Busy` | unit core, int per driver |
| F-DB02-N2 | syntax error keeps the connection usable | int per driver |

### F-DB03 Cancel and timeouts
| id | case | where |
|---|---|---|
| F-DB03-H1 | cancel a sleeping statement from another task | int pg, sqlite, odpi |
| F-DB03-H2 | 1 s timeout ends a long statement with `Timeout` | unit core helper, int per driver, diagnose `--deep` |
| F-DB03-E1 | cancel when idle is harmless | int per driver |
| F-DB03-E2 | thin Oracle: abandoned call marks the connection lost; reconnect works | int thin |

### F-DB04 Browse schema metadata
| id | case | where |
|---|---|---|
| F-DB04-H1 | schemas, objects by kind, columns, indexes, constraints, source | int per dialect, sqlite unit |
| F-DB04-E1 | mixed-case and quoted names, names with spaces | int per dialect |
| F-DB04-E2 | Oracle dictionary SQL valid on 19c | review + issue #9 |
| F-DB04-N1 | no privilege on a view: listed as a warning, not a crash | diagnose `dictionary` step |

### F-DB05 Manage connections and passwords
| id | case | where |
|---|---|---|
| F-DB05-H1 | add, list, show, remove profiles | unit `service`, `ops`, cli |
| F-DB05-H2 | store, check and delete a password in the OS store | `secrets` ignored test, cli |
| F-DB05-E1 | unknown keys and inline passwords in the profile file rejected | unit `profiles` |
| F-DB05-E2 | KeePass store; import of PyCharm data sources | issue #7 |

## Performance
`db-bench` per backend, numbers in `replica/perf.md`.
