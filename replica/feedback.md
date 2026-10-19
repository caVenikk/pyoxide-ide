# Database client: what users of the reference tools report

Public feedback about the database tools of the reference IDE (also sold as a
standalone database IDE on the same engine). Collected on 2026-10-09 for the
areas `db` and `db-gui`. All wording below is a paraphrase; the raw rows with
links live in the local, git-ignored `replica/reviews.csv`.

## Sample and method

80 items from four sources, 73 of them dated 2023 or later and 40 dated 2025 or
later. One item is one issue, thread or comment.

| source | items | what it is |
|---|---|---|
| JetBrains issue tracker (project DBE, which also holds the PyCharm database tools) | 41 | 18 open, 23 resolved at collection time; votes are issue votes, not distinct people |
| Hacker News comments | 16 | via the Algolia search API |
| JetBrains community forum (DataGrip and PyCharm sections) | 14 | support-style threads |
| Stack Overflow questions | 9 | tags `datagrip`, `pycharm`, `oracle` |

Not reached or not used:
- Reddit (r/pycharm, r/datagrip, r/Python, r/SQL): the unauthenticated JSON
  endpoint returned a block page and the in-app browser refused the domain.
  Nothing was collected and nothing was worked around.
- X/Mastodon: not attempted.
- Blog posts: searched, but the comparison articles found were written by
  vendors of competing tools, so none is used as evidence. The vendor's own blog
  is used only to date upstream changes (see "Already addressed upstream").
- No ratings exist in these sources, so there is no star weighting. Themes were
  assigned by hand (an item can belong to several themes). The bundled
  `reviews.py` ranker was run with database-specific themes as a cross-check;
  its keyword matching was too coarse to use for the counts below.

Counts below are items per theme out of 80. A theme with fewer than 3 items or a
single source is marked thin.

## 1. What they hate

| # | theme | items | sources | since 2025 | links | what users say, in short |
|---|---|---|---|---|---|---|
| 1 | The IDE freezes or lags in the database tool window and result grid | 17 | 4 | 12 | [DBE-25765](https://youtrack.jetbrains.com/issue/DBE-25765), [DBE-26735](https://youtrack.jetbrains.com/issue/DBE-26735), [DBE-23783](https://youtrack.jetbrains.com/issue/DBE-23783) | Grids with large JSON or text cells, SSH-tunnel settings edits and several 2025 releases block the UI for seconds to minutes. One 2026 freeze ticket was still open at collection time with 102 automatic reports from 49 users. |
| 2 | Updates break or lose things | 12 | 3 | 11 | [DBE-23860](https://youtrack.jetbrains.com/issue/DBE-23860), [forum thread on consoles](https://intellij-support.jetbrains.com/hc/en-us/community/posts/31715678282770-Last-update-messed-up-SQL-consoles-big-time-any-chance-to-revert), [DBE-26862](https://youtrack.jetbrains.com/issue/DBE-26862) | After upgrades, result tabs stayed empty, consoles lost their connection, data sources vanished and saved passwords were cleared. |
| 3 | Oracle connectivity | 11 | 3 | 5 | [DBE-25762](https://youtrack.jetbrains.com/issue/DBE-25762), [DBE-21622](https://youtrack.jetbrains.com/issue/DBE-21622), [DBE-25760](https://youtrack.jetbrains.com/issue/DBE-25760) | Java runtime bumps broke TLS and Kerberos logins; driver files went missing, including on machines without internet; wallet and TNS setups need manual steps; service name versus SID is easy to get wrong. |
| 4 | Console, query-file and result-tab workflow | 9 | 4 | 4 | [forum thread on consoles](https://intellij-support.jetbrains.com/hc/en-us/community/posts/31715678282770-Last-update-messed-up-SQL-consoles-big-time-any-chance-to-revert), [Stack Overflow on default transaction mode](https://stackoverflow.com/questions/77870131) | The 2025.3 console and query-file redesign scattered saved queries; result tabs sometimes stay empty or are not kept across restarts; manual-transaction mode resets every session; statements in one console run strictly one after another. Six items since 2023, the rest are older open requests. |
| 5 | Introspection is slow, hangs or is forgotten | 9 | 3 | 4 | [DBE-17125](https://youtrack.jetbrains.com/issue/DBE-17125), [DBE-25205](https://youtrack.jetbrains.com/issue/DBE-25205), [DBE-26529](https://youtrack.jetbrains.com/issue/DBE-26529) | The schema is re-read after every restart; an Oracle schema with about 14,000 tables fails on temp space; a PostgreSQL introspection never finishes although another client loads the same database through the same tunnel immediately. |
| 6 | Memory hunger and out-of-memory on big data | 9 | 4 | 3 | [DBE-18168](https://youtrack.jetbrains.com/issue/DBE-18168), [DBE-17885](https://youtrack.jetbrains.com/issue/DBE-17885), [forum thread on export memory](https://intellij-support.jetbrains.com/hc/en-us/community/posts/10236240739474-DataGrip-using-too-much-memory-when-exporting-to-file) | The tool holds about 2 GB while idle in two reports, and exporting a multi-million-row table exhausts the heap even after raising the limit. |
| 7 | PostgreSQL gaps | 9 | 4 | 6 | [DBE-4377](https://youtrack.jetbrains.com/issue/DBE-4377), [DBE-6155](https://youtrack.jetbrains.com/issue/DBE-6155), [DBE-22470](https://youtrack.jetbrains.com/issue/DBE-22470) | No procedural-language debugger, no psql meta-commands or psql launcher, no clear route to a Unix socket or a path-style URL, Kerberos on Windows is hard to set up, and introspection hangs on some servers. |
| 8 | SQLite feels second-class | 8 | 3 | 5 | [DBE-25093](https://youtrack.jetbrains.com/issue/DBE-25093), [DBE-19874](https://youtrack.jetbrains.com/issue/DBE-19874), [DBE-24764](https://youtrack.jetbrains.com/issue/DBE-24764) | Quick inspection of a database file needs a project; sqlite3 dot-commands do not run; the SSH tab is offered but does not work; the bundled engine lagged behind upstream SQLite for a long time. |
| 9 | Oracle dialect gaps against SQL*Plus and PL/SQL | 6 | 3 | 2 | [DBE-689](https://youtrack.jetbrains.com/issue/DBE-689), [forum thread on EXEC](https://intellij-support.jetbrains.com/hc/en-us/community/posts/34439578046226-Oracle-PL-SQL-EXEC-Keyword-Stops-Script-From-Running) | Scripts that use DESCRIBE or EXEC fail in the console; stored-procedure output through cursors is not shown; native JSON columns can crash the data viewer; two threads call Oracle support weak. |
| 10 | Driver download and management | 6 | 2 | 2 | [DBE-18228](https://youtrack.jetbrains.com/issue/DBE-18228), [DBE-24975](https://youtrack.jetbrains.com/issue/DBE-24975), [forum thread](https://intellij-support.jetbrains.com/hc/en-us/community/posts/21041076339602-Driver-class-org-postgresql-Driver-not-found) | Drivers reported as missing although downloaded; no way to point to an internal mirror; a frozen dialog when the network drops during a driver update; a native SQLite library not found on Windows. |
| 11 | Too heavy for simple jobs, lighter tools win | 6 | 2 | 6 | [HN comment](https://news.ycombinator.com/item?id=47329745), [HN comment](https://news.ycombinator.com/item?id=44074015), [Stack Overflow](https://stackoverflow.com/questions/77257254) | Users call the tool big, slow and memory-hungry for quick queries, and some switched to lighter clients or wrote their own terminal tools. Mostly comments, no measurements. |
| 12 | Passwords and credentials | 4 | 2 | 2 | [DBE-22377](https://youtrack.jetbrains.com/issue/DBE-22377), [DBE-26862](https://youtrack.jetbrains.com/issue/DBE-26862) | Credentials must be duplicated into a second store instead of read from the user's password manager; saved passwords disappear on update. |
| 13 | SSH tunnels | 3 | 2 | 3 | [DBE-23783](https://youtrack.jetbrains.com/issue/DBE-23783), [HN comment](https://news.ycombinator.com/item?id=49980931) | Tunnelled connections freeze the settings dialog or force restarting the app. |

Thin: SSH tunnels (3 items, small), project coupling (1 item), external secret
managers (1 item), Windows-specific and startup-specific database complaints
(see "Gaps in the evidence").

## 2. What is missing (features asked for by name)

| # | request | items | links | note |
|---|---|---|---|---|
| 1 | Show results of a SQL script launched from a run configuration | 1 | [DBE-10653](https://youtrack.jetbrains.com/issue/DBE-10653) | 125 votes, open since 2020; relevant because Python projects keep SQL next to code. |
| 2 | Run several statements concurrently from one console | 1 | [DBE-5691](https://youtrack.jetbrains.com/issue/DBE-5691) | 72 votes, open since 2018. |
| 3 | psql meta-commands and a stored-procedure debugger for PostgreSQL | 2 | [DBE-6155](https://youtrack.jetbrains.com/issue/DBE-6155), [DBE-4377](https://youtrack.jetbrains.com/issue/DBE-4377) | 85 and 147 votes, both open for years. |
| 4 | SQL*Plus DESCRIBE and EXEC in the Oracle console | 1 | [DBE-689](https://youtrack.jetbrains.com/issue/DBE-689) | 111 votes, open since 2013. |
| 5 | Grid and export features: parquet export, date format control for import and export, column pinning, raw SQL values in cells | 4 | [DBE-16632](https://youtrack.jetbrains.com/issue/DBE-16632), [DBE-7569](https://youtrack.jetbrains.com/issue/DBE-7569), [DBE-26267](https://youtrack.jetbrains.com/issue/DBE-26267) | Parquet (55 votes) and date formats (42) are open; pinning (62) and raw SQL values (63) were delivered upstream in 2026. |
| 6 | Credentials from an external password manager | 1 | [DBE-22377](https://youtrack.jetbrains.com/issue/DBE-22377) | Thin, one request. |
| 7 | Use a database file or connection without creating a project first | 1 | [DBE-25093](https://youtrack.jetbrains.com/issue/DBE-25093) | Thin, but matches the comments about the tool being too heavy for quick jobs. |
| 8 | A custom repository for driver downloads, offline driver setup | 2 | [DBE-18228](https://youtrack.jetbrains.com/issue/DBE-18228), [DBE-21622](https://youtrack.jetbrains.com/issue/DBE-21622) | Matters only for JDBC-based clients. |

## 3. What is unsolved (whole jobs the reference serves badly)

1. **Huge schemas.** Tens of thousands of objects on Oracle, or one slow
   catalog on PostgreSQL, still lead to multi-minute or never-ending
   introspection and memory exhaustion ([DBE-25205](https://youtrack.jetbrains.com/issue/DBE-25205),
   [HN comment](https://news.ycombinator.com/item?id=38307304),
   [DBE-26529](https://youtrack.jetbrains.com/issue/DBE-26529)). 5 items, 3 sources.
2. **Quick look at a database.** Opening one SQLite file or one connection for
   a minute is slower than with lightweight clients ([DBE-25093](https://youtrack.jetbrains.com/issue/DBE-25093),
   [HN comment](https://news.ycombinator.com/item?id=44074015),
   [Stack Overflow](https://stackoverflow.com/questions/79386311)). Small: 3 items, 3 sources, mostly anecdotal.
3. **Oracle environments that depend on the Java runtime.** Recent JDK bumps
   broke TLS or Kerberos for some users, and driver files go missing on
   locked-down machines ([DBE-25762](https://youtrack.jetbrains.com/issue/DBE-25762),
   [DBE-25760](https://youtrack.jetbrains.com/issue/DBE-25760), [DBE-21622](https://youtrack.jetbrains.com/issue/DBE-21622)).
   Thin: 4 items, all from the issue tracker.
4. **Parity with command-line clients.** Scripts written for psql, sqlite3 and
   SQL*Plus fail in the console because their meta-commands are not understood
   ([DBE-6155](https://youtrack.jetbrains.com/issue/DBE-6155), [DBE-19874](https://youtrack.jetbrains.com/issue/DBE-19874),
   [DBE-689](https://youtrack.jetbrains.com/issue/DBE-689)). 5 items, 2 sources.
5. **Trust in upgrades.** Users keep many connections and consoles and expect
   them to survive an update ([forum thread](https://intellij-support.jetbrains.com/hc/en-us/community/posts/21153132214546-Lost-Datasources-on-Upgrade),
   [DBE-23571](https://youtrack.jetbrains.com/issue/DBE-23571)). 12 items, 3 sources (theme 2 above).

## What users praise (keep this in mind)

Three comments from one source (thin) value the breadth of supported
databases, schema-aware completion, many export options, result diffing,
inline results and the depth of the tool, and one says it made Oracle work
bearable. Whatever Pyoxide does better on speed has to keep schema-aware
completion usable, or users will treat it as a downgrade.

## Already addressed upstream (do not treat as open)

- The 2025.3 redesign of query consoles and query files was rolled back by the
  vendor in 2025.3.1 ([vendor blog, 2025-12-18](https://blog.jetbrains.com/datagrip/2025/12/18/query-consoles-are-coming-back/)).
- Since 2023.3 the introspection level is chosen automatically per schema
  based on size ([vendor blog, 2023-12-06](https://blog.jetbrains.com/datagrip/2023/12/06/introspection-level-auto-selection/)).
- Fixed in releases: the grid freeze with large cells (2026.1.1,
  [DBE-25765](https://youtrack.jetbrains.com/issue/DBE-25765)), the SSH-edit freeze (2025.2.2,
  [DBE-23783](https://youtrack.jetbrains.com/issue/DBE-23783)), empty result tabs (2025.2.1,
  [DBE-23860](https://youtrack.jetbrains.com/issue/DBE-23860)). A newer per-cell rendering freeze
  ([DBE-26735](https://youtrack.jetbrains.com/issue/DBE-26735)) was still open.

## Gaps in the evidence

- Nothing collected is specific to Windows performance of the database tools.
  Do not claim a Windows advantage from this data; measure it
  (`replica/perf.md`).
- Startup of the database tool window specifically has two weak items (memory
  right after launch, and a general startup comment). Treat as unproven.
- Oracle thin versus OCI is discussed only indirectly; users mostly hit JDBC
  driver, wallet and Java runtime problems. Kerberos with the pure-Rust thin
  driver is a known limitation (ADR 0003).
- Most comments are not measurements. Where numbers appear (idle memory over
  2 GB, 14,000-table schemas), they are single reports.

## Implications for Pyoxide

### Existing rows in `replica/features/`

| theme | existing rows | suggested change |
|---|---|---|
| 1, 6 Freezes and memory | Data grid with paging, Paginated results, Typed result values, Export results (CSV / JSON) | Keep as `must`. Add the grid and large-value rows below and give them budgets in `docs/performance.md` (frame time while scrolling a 100k-row page with large cells; memory while exporting 10 million rows). |
| 3, 9, 10 Oracle | Oracle connection (thin driver), Oracle connection (Oracle Client / ODPI-C), TLS and wallets, Oracle TNS alias and connect descriptors, Connection diagnostics, Test connection | The design without JDBC drivers or a Java runtime removes theme 10 and most of theme 3. Raise Connection diagnostics in importance: map ORA- and TNS- codes to a cause and a fix. |
| 5 Introspection | Schema introspection (tables / views / columns), Indexes constraints sequences procedures, Schema and object filters | Raise Schema and object filters from `could` to `should`. Add the two introspection rows below. |
| 2, 12 Upgrades and secrets | Connection profiles with keychain secrets, Password storage in KeePass (kdbx), Import data sources and passwords from PyCharm | Keep. The import row removes a switching barrier; keep it `should`. |
| 4 Console | SQL console with history, Transaction mode (auto / manual), Query timeout and cancel | Persist transaction mode per profile (users report it resets). |
| 13 SSH | SSH tunnel (`could`, after MVP) | Raise to `should`; the real requirement is that tunnel setup never blocks the UI. |
| 7, 8 PostgreSQL and SQLite | PostgreSQL connection, SQLite connection | Add the rows below. |

### Proposed new rows

| feature | area | priority | reason |
|---|---|---|---|
| Virtualized result grid (render only visible cells) | db-gui | must | Theme 1: per-cell rendering of large values is the most voted freeze. |
| Large value policy for LOB, JSON and text (lazy fetch with byte cap) | database | must | Themes 1 and 6: huge cells freeze the grid and exhaust memory. |
| Streaming export without buffering the whole result | database | should | Theme 6: exports of millions of rows crash on memory. |
| Persistent schema cache across restarts | database | must | Theme 5: the schema is re-read after each restart or window switch. |
| Cancellable, scoped introspection with progress | database | must | Theme 5 and gap 1: huge schemas and hangs; the user must be able to stop and narrow it. |
| Oracle connect-descriptor builder (service name, SID, TNS alias, EZConnect) with validation | database | should | Theme 3: SID versus service name confusion and wrong URL separators. |
| Oracle Kerberos and external authentication via the Oracle Client backend | database | could | Themes 3 and gap 3; thin driver has no Kerberos (ADR 0003). |
| SQL*Plus commands in the console (DESCRIBE, EXEC) | db-gui | should | Theme 9: 111 votes, scripts fail. |
| PostgreSQL Unix-socket connections | database | should | Theme 7: cheap to support, no clear route in the reference. |
| Console meta-commands for psql and sqlite3 (subset) | db-gui | could | Gap 4: scripts for the command-line clients fail. |
| Open a SQLite file or a profile directly (command line, drag and drop) without a project | db-gui | should | Theme 8 and gap 2: quick inspection should take seconds. |
| Run a SQL file as a run configuration with a result grid | db-gui | should | Missing 1: 125 votes; fits a Python IDE. |
| Concurrent statements per connection | db-gui | could | Missing 2: 72 votes. |
| Column pinning in the grid | db-gui | should | Missing 5: 62 votes before the reference added it. |
| Date and locale format settings for grid, import and export | db-gui | should | Missing 5: 42 votes, open. |
| Parquet export | database | could | Missing 5: 55 votes, open. |
| Profile and console storage that is never rewritten destructively (versioned format, migration tests) | platform | should | Theme 2: lost data sources, consoles and passwords after updates. |
| External secret provider (command or password-manager CLI) | database | could | Theme 12: thin, one request. |

## Positioning for the README (draft)

Pyoxide is a native IDE for Python with a built-in database client for Oracle,
PostgreSQL and SQLite, built for people who want database work to stay out of
their way. It runs without a Java runtime, so there is no driver download, no
Java version to match with an Oracle server, and no heap limit to tune. The
default Oracle driver is a pure Rust implementation of the thin protocol,
with an Oracle Client fallback for environments that need it; connection
profiles are plain text files that are easy to back up and share, with
passwords kept in the system keychain or a KeePass file. Queries are paged and
cancellable and large values are loaded on demand, so the interface is designed
never to wait for the database, a big cell or a huge schema. The same engine
serves the IDE and the command line. Startup time, memory use and result-grid
responsiveness, including on Windows, are tracked against budgets in
`docs/performance.md`, and any number quoted here comes from
`replica/perf.md`.

Notes for whoever publishes this paragraph:
- The statements about Rust, the absence of a Java runtime and driver
  downloads, plain-text profiles and the keychain or KeePass storage describe
  accepted design (ADR 0003 and ADR 0004). Paging, cancel, on-demand loading
  and PostgreSQL and SQLite support are not complete in `replica/features/`;
  describe them as shipped only when they are.
- The last sentence is the only place numbers would appear. Do not state any
  speed, startup, memory or Windows figure until the measured rows exist.
- Do not quote users, and do not name the reference products in the README.
