# SQL navigation, usages, rename and structure

## Steps
1. In a file bound to a data source whose model has `customers (id, name)` and `orders (id, customer_id, total)`, type `select c.name from customers c where c.id > 1 order by c.name`.
2. Put the caret on `c` of `c.name` and invoke Go to Declaration.
3. Put the caret on `customers` in `FROM` and invoke Go to Declaration; do the same on `name` of `c.name`.
4. Put the caret on the alias `c` in `FROM` and look at the highlighted usages; invoke Rename and enter `cust`.
5. Type `with recent as (select total as amount from orders) select r.amount from recent r order by amount`; invoke Go to Declaration on `recent` in `FROM`, on `r.amount` and on `amount` in `ORDER BY`; rename `amount`.
6. Type `create table staging (id int, note text); select s.note from staging s;` and invoke Go to Declaration on `staging` and on `note` of `s.note`; rename `staging`.
7. In a PL/SQL procedure with a parameter and a variable, invoke Go to Declaration on both and rename the variable; do the same in a PL/pgSQL function body.
8. Invoke Rename on `customers`.
9. Open the file structure.

## Expected
- Step 2: the caret goes to the alias `c` in `FROM`; on the alias itself, the usages show.
- Step 3: the Database tool window opens with the table `public.customers` selected; `name` selects that column under the table's columns.
- Step 4: the alias in `FROM` and its three uses are highlighted; after Rename every one of them reads `cust`.
- Step 5: the caret goes to the CTE name, to the select item `amount` of the CTE, and to the select item `amount`; renaming changes the item and the uses, in `ORDER BY` as well.
- Step 6: the caret goes to the `create table` name and to the column `note` of the statement; renaming `staging` changes the statement and every use.
- Step 7: parameters, variables, cursors and loop variables go to their declarations; a loop variable that hides a variable is not renamed with it; the rename also reaches the body in `$$ .. $$`.
- Step 8: nothing is renamed; the message says that a database object is renamed in the database.
- Step 9: the statements appear with their kind and name (`staging` table with its columns, `select` with its CTEs, `insert into staging`), packages with their procedures and functions.

## Source
Public documentation of navigation, usages and refactoring in the reference IDE's SQL support (verify the list of renamable elements and the structure view).
