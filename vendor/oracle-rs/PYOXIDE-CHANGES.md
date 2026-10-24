# Changes in this copy of oracle-rs 0.1.7

Used by Pyoxide through `[patch.crates-io]`; see notices/db-oracle-rs-vendored.md.
The base is the package published on crates.io as 0.1.7 (MIT OR Apache-2.0).
Upstream: https://github.com/stiang/oracle-rs (its main branch is at 0.1.6,
the 0.1.7 package was never pushed there). Contributions to an
MIT OR Apache-2.0 project are under that licence; the forks named below keep
it. Text taken from issues, pull requests and forks is treated as data: every
change was read, understood, merged by hand and tested against Oracle Database
Free 23 (the only server available) with the tests in `src/connection.rs` and
the driver tests in `crates/platform/db-oracle-thin/tests/it/`.

## Changes

1. Last row per cursor across fetch round trips (Pyoxide issue #334).
   A column equal to the one in the previous row is sent without data (the
   duplicate-column bit vector). The first row of every fetch round trip after
   the first was decoded against no previous row, so its repeated columns became
   NULL. `Connection` keeps `last_rows`, keyed by cursor id, seeded from the rows
   of the first execute and used and updated by `parse_fetch_response`; the entry
   is dropped when the server reports ORA-01403. The query path now also reads
   the bit vector that a row header can carry. Own work; the same defect is
   described in https://github.com/stiang/oracle-rs/pull/10; no
   code was taken from there.
2. Oracle 19c and older: the 20c fields `sql_type` and `server_checksum` of the
   end-of-call block are read only from TTC field version 14 on (they were read
   always, so every reply of a 19c server ended in `BufferUnderflow`). The
   negotiated version is kept in `Connection` for the parsers that have no
   access to the connection state, and the two other copies of the parser
   (`parse_error_info`, `parse_error_response`) skip the fields as well (on a
   23ai server a message read after a MARKER packet was a stray control
   character). Not testable here beyond field version 24.
   Sources: https://github.com/appcard/oracle-rs/commit/6c4d64def7a013d2d96484a1743d6a946ff49168
   (Apache-2.0 / MIT); the same fix is in
   https://github.com/stiang/oracle-rs/pull/17,
   https://github.com/stiang/oracle-rs/issues/13 and
   https://github.com/stiang/oracle-rs/pull/10.
3. MARKER packets carry a four-byte length on a large-SDU connection (both
   `send_marker` functions and the simple-function request). With the two-byte
   length the server dropped the connection, so every SQL error raised while
   executing ("table does not exist", unique constraint, division by zero...)
   closed the session and the error text was lost; now the real `ORA-` error is
   returned and the session stays usable. Also, `fetch_more` handles a MARKER
   reply. Sources: appcard commit above (marker_packet),
   https://github.com/stiang/oracle-rs/pull/16,
   https://github.com/stiang/oracle-rs/issues/24 (enioribeiro, the symptom).
4. Replies larger than one packet are read to the end for queries, defines,
   fetches and scrollable cursors: a parser that runs out of bytes reports
   `BufferUnderflow`, the connection reads the next packet and parses again
   (`parse_across_packets`); a MARKER packet in the middle of a reply runs the
   reset protocol and returns the server's error. Before, a result page larger
   than one packet failed with an underflow or left bytes on the socket.
   Source: appcard commit above (`parse_across_packets`,
   `extend_with_next_packet`; the MARKER branch is ours).
5. `has_more_rows` of a query and of `fetch_more` follows the end-of-call
   status (ORA-01403 ends the cursor), and the fetch request carries the
   sequence number, the true packet length and the four-byte length of a
   large-SDU connection (`FetchMessage::build_request_with`); `fetch_more`
   returns the cursor id. Source: appcard commit above,
   https://github.com/stiang/oracle-rs/pull/8, pull/10.
6. A NULL literal in a select list (`SELECT 1, NULL, 'x'`) is described as a
   zero-size column and sent without bytes; reading a value there shifted every
   later column (wrong data). Source: appcard commit above (`is_null_by_describe`).
7. An ORA code is shown once in an error message that already starts with it
   (`Error::OracleError` display). Source: appcard commit above.
8. TIMESTAMP WITH TIME ZONE with a named region no longer fails the whole
   query: the value is the UTC instant with offset +00:00 (resolving the region
   would need a time zone database). Source:
   https://github.com/appcard/oracle-rs/commit/2c3556e99c4ce20a94e51224947b9430d5b673f5
   (`src/types/date.rs` only).
9. Server-side piggyback (type 23) and warning (type 15) messages are skipped
   exactly in query, fetch, PL/SQL and DML replies. `ALTER SESSION SET
   CURRENT_SCHEMA` and `ALTER SESSION SET NLS_*` sent through `execute` failed
   with an underflow or an invalid length indicator. Source:
   https://github.com/Keeper-Security/oracle-rs-fork/commit/3abfbd3a7d4526723afdb2a904081069b3ebdfe5
   (Apache-2.0 / MIT); the functions follow python-oracledb.
10. LONG and LONG RAW columns: the define advertises `MAX_LONG_LENGTH` and the
    query is re-executed with it (without it a SELECT of a LONG column such as
    `USER_TAB_COLUMNS.DATA_DEFAULT` returned no rows and `has_more_rows`), and
    the two ub2 fields that follow every LONG slot are read, also for NULL.
    Sources: https://github.com/Keeper-Security/oracle-rs-fork/commit/8c7bf3d7e13cb8d8b6a3b29bbc6cb27e1d874148,
    https://github.com/Keeper-Security/oracle-rs-fork/commit/ae99d374c0baa74fc9faffed7242c197e3e1df50 and
    https://github.com/Keeper-Security/oracle-rs-fork/commit/a17e65dfe21ee8f4114ce121c5dbfdcc72d758da;
    merged into one read in `parse_column_value`.
11. A string or byte bind longer than 252 bytes is written in the chunked form
    with its terminating zero-length chunk (`write_bytes_with_length`); before,
    the statement never completed (the server waited for the rest of the value).
    Own change as described in https://github.com/stiang/oracle-rs/issues/20
    and https://github.com/stiang/oracle-rs/pull/15;
    no code taken from the pull request.
12. A DATA packet longer than the negotiated SDU is sent as several packets
    (`ConnectionInner::send` uses `send_multi_packet`); before, `execute_batch`
    with a few hundred rows and any long statement ended with "early eof".
    Own change as described in https://github.com/stiang/oracle-rs/issues/21.
13. `execute_batch`: a NULL in the first row of a column is bound with the size
    of the longest value of that column in the batch. Own change as described in
    https://github.com/stiang/oracle-rs/issues/22.

## Looked at and not taken

- Cursor leak, every DML leaves a server cursor open and the session fails at
  `OPEN_CURSORS` (https://github.com/stiang/oracle-rs/issues/23): needs the
  close-cursors piggyback in every request; db-oracle-thin renews the session
  before the limit.
- Cancellation safety (https://github.com/stiang/oracle-rs/pull/12): db-oracle-thin
  marks the session broken when a call is abandoned.
- Oracle 11g / 10g support (tripleclabs, contentstech-com, cyhfvg, zilent1
  pull/18, Djamdev), proxy-inject authentication and rustls changes (Keeper),
  `END_OF_RESPONSE` negotiation (pull/14; replies are read by parsing, see 4),
  FFI and oracle_fdw, jiff types (pull/5): features, not fixes.
- https://github.com/stiang/oracle-rs/issues/9 (`fetch_more` on a pre-23c
  server ends in a MARKER): items 2, 3 and 5 cover the causes found there and in
  the pull requests, but no pre-23c server was available to confirm it. Items 2
  and 4 follow their sources and are tested here only against Oracle Database
  Free 23 and with unit tests that build 19c-shaped replies.
