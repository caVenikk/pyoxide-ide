# SQL completion

## Steps
1. Bind a console or a `.sql` file to a data source whose model has `customers (id, name, email)` and `orders (id, customer_id, total)` with a foreign key `orders.customer_id -> customers.id`. Disconnect the data source.
2. Type `select c. from customers c` with the caret after `c.`; invoke completion.
3. Type `select * from orders o join customers c on ` and invoke completion.
4. Type `select * from ` and invoke completion; then complete a table that exists in another schema.
5. Type `insert into customers (` and invoke completion; type `update customers set ` and invoke completion.
6. Rename a table to a mixed-case name in the model (`"Order Items"`), complete it in `from`.

## Expected
- Step 2: the columns of `customers` with their types, although `FROM` comes after the caret and the data source is not connected.
- Step 3: the join condition `o.customer_id = c.id` is offered as one item, then the columns of `o` and `c`.
- Step 4: tables, views and CTEs of the scope's schema first; a table of another schema is inserted with its schema name.
- Step 5: columns of `customers` that are not listed yet; columns of `customers` after `set`.
- Step 6: the name is inserted quoted.
- Keywords offered are only those valid at the caret and follow the keyword case of the code style.

## Source
Public documentation of code completion in SQL (verify the join condition item and the alias option against the reference).
