# Typed values

## Steps
Select one value of each type: integer, a 38-digit decimal, float, text with
non-ASCII characters, empty string, NULL, date, timestamp, timestamp with time
zone, interval, binary, boolean (PostgreSQL, SQLite integer), UUID and JSON
(PostgreSQL), CLOB/BLOB larger than 1 MiB (Oracle).

## Expected
- Numbers keep every digit; text keeps characters; NULL is distinct from an
  empty string (except Oracle, where '' is NULL).
- Dates and timestamps keep fractional seconds and offsets.
- Large LOBs come back truncated with the full length known.

## Source
Observed behaviour of the data grid; type lists from the public Oracle,
PostgreSQL and SQLite documentation.
