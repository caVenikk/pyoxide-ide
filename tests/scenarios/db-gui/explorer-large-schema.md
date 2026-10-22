# A schema with 10 000 tables

## Steps
1. Create 10 000 empty tables in one schema (a generated script).
2. Connect and expand the schema; scroll; use speed search.
3. Restart the IDE and open the tree before connecting.

## Expected
- Expansion and scrolling stay smooth; the UI thread never waits on the database.
- The first load shows progress (database, schema) and can be cancelled; after cancel the previous model is intact.
- After a restart the tree is shown from the stored model at once, before any connection.

## Source
performance budget (docs/performance.md); observed
