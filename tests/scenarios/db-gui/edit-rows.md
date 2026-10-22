# Edit rows in the grid (later milestone)

## Steps
1. Open a table with a primary key; change a cell, add a row, delete a row.
2. Open the SQL preview, then Submit.
3. Repeat on a table without a primary key containing two identical rows; change one of them.
4. Make a change, then Revert.

## Expected
- Changes stay local and are marked (changed cell, struck-through row, new row) until Submit; Revert drops them without touching the database.
- Submit sends deletes, then updates, then inserts; an update sets only changed columns; the page is reloaded afterwards.
- Without a primary key the row is identified by all comparable columns (on PostgreSQL also by the physical row id); if the number of affected rows is not the expected one the user gets a warning and the changes are rolled back.
- Read-only data sources and result sets that map to no single table refuse editing and say why.

## Source
observed; recon open question on the submit SQL
