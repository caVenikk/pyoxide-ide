# SQL navigation, usages, rename and structure

## Steps
1. In a file bound to a data source whose model has `customers (id, name)` and `orders (id, customer_id, total)`, type `select c.name from customers c where c.id > 1 order by c.name`.
2. Put the caret on `c` of `c.name` and invoke Go to Declaration.
3. Put the caret on `customers` in `FROM` and invoke Go to Declaration; do the same on `name` of `c.name`.
4. Put the caret on the alias `c` in `FROM` and look at the highlighted usages; invoke Rename (Shift+F6) and enter `customer_alias` in the editor. Cancel, repeat the rename, then confirm and undo.
5. Type `with recent as (select total as amount from orders) select r.amount from recent r order by amount`; invoke Go to Declaration on `recent` in `FROM`, on `r.amount` and on `amount` in `ORDER BY`; rename `amount`.
6. Type `create table staging (id int, note text); select s.note from staging s;` and invoke Go to Declaration on `staging` and on `note` of `s.note`; rename `staging`.
7. In a PL/SQL procedure with a parameter and a variable, invoke Go to Declaration on both and rename the variable; do the same in a PL/pgSQL function body.
8. Invoke Rename on `customers`.
9. Open the file structure (Cmd+F12), type a part of a name, and choose a row. In a new profile, enable Select statements to show a file containing a single SELECT. Use the gear menu to toggle alphabetical sorting and search narrowing, then reopen File Structure. Drag the title or resize from an edge.

## Expected
- Step 2: the caret goes to the alias `c` in `FROM`; on the alias itself, the usages show.
- Step 3: the Database tool window opens with the table `public.customers` selected; `name` selects that column under the table's columns.
- Step 4: the alias in `FROM` and its three uses are highlighted in the editor (the alias itself as a write); the editor selects `c` in place. All four show `customer_alias` while typing. Escape restores the original names. Enter commits all four occurrences and one Undo restores them together.
- Step 5: the caret goes to the CTE name, to the select item `amount` of the CTE, and to the select item `amount`; renaming changes the item and the uses, in `ORDER BY` as well.
- Step 6: the caret goes to the `create table` name and to the column `note` of the statement; renaming `staging` changes the statement and every use.
- Step 7: parameters, variables, cursors and loop variables go to their declarations; a loop variable that hides a variable is not renamed with it; the rename also reaches the body in `$$ .. $$`.
- Step 8: no dialog opens; a hint at the caret says that a database object is renamed in the database.
- Step 9: the popup opens with the symbol at the caret selected; typing narrows the rows; Enter moves the caret to the symbol's name. DDL is initially enabled. A table has nested columns and types. Additional groups show SQL statement previews and retain their selected state. Sorting preserves the tree hierarchy. Enter or F4 navigates to the selected item. With Select disabled the popup stays open and shows “Structure is empty”. Re-enabling Select restores the row.

## Source
Public documentation of navigation, usages and refactoring in the reference IDE's SQL support (verify the list of renamable elements and the structure view).
