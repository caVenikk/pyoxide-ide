# SQL resolution, inspections and quick fixes

## Steps
1. In a file bound to a data source, type `select * from custmers`; wait for the highlighting delay.
2. Open the intention popup on the mark and choose the change to the similar name.
3. Type `select id from orders o join customers c on o.customer_id = c.id` (both tables have `id`).
4. Type `delete from orders`; type `update orders set total = 0`.
5. Type `select * from orders where total = null`.
6. Type `with unused as (select 1) select 2`.
7. Detach the file from every data source; type `select * from nothing`.
8. Load a schema to names only (no columns) and type `select anything from <table of that schema>`.
9. Choose "Suppress for statement" on a mark.

## Expected
- Step 1: `custmers` is marked as unresolved; step 2 offers "Change to 'customers'" and applies it.
- Step 3: `id` is marked ambiguous; the fixes qualify it with `o` or `c`.
- Step 4: both statements are warned about a missing `WHERE`; the fix adds `where ` and places the caret after it.
- Step 5: `= null` is warned about; the fix writes `is null`.
- Step 6: the unused CTE is greyed; the fix removes it.
- Step 7: no unresolved mark (no scope); syntax errors still show.
- Step 8: no unresolved mark for the column (unknown, not unresolved).
- Step 9: a `-- noinspection` comment is added before the statement and the mark disappears.

## Source
Public documentation of SQL inspections and resolution scopes (verify the list, default severities and texts against the reference).
