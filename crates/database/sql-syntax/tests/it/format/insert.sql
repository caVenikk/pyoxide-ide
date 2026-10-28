insert into orders (customer_id, total, status) select id, 0, 'new' from customers where active on conflict (customer_id) do nothing returning id;
=====
insert into orders (customer_id, total, status)
select id, 0, 'new'
from customers
where active
on conflict (customer_id) do nothing
returning id;
