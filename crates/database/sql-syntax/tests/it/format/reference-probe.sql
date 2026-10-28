select a, b, c from demo.orders where status = 'paid' and total > 10 and customer_id in (1, 2, 3) order by placed_at desc;

select o.id, o.total, c.name, c.email, c.country, o.status, o.placed_at, o.shipped_at, o.client_ip, o.invoice_no from demo.orders o join demo.customers c on c.id = o.customer_id left join demo.order_items i on i.order_id = o.id where o.total > (select avg(total) from demo.orders where status = 'paid') group by o.id, c.name having count(*) > 1;

select id,
       -- own-line comment inside the select list
       total
from demo.orders
where status = 'paid' -- trailing comment
  and total > 10;

select id, case when total > 100 then 'big' when total > 10 then 'mid' else 'small' end as size from demo.orders;

with recent as (select customer_id, count(*) as n from demo.orders where placed_at > now() - interval '30 days'
group by customer_id), vip as (
select customer_id
from recent
where n > 3)
select c.*
from demo.customers c join vip v
on v.customer_id = c.id;

insert into demo.categories (id, name, parent_id) values (100, 'probe', null), (101, 'probe 2', 100);

update demo.orders set status = 'cancelled', shipped_at = null where id in (select order_id from demo.order_items where qty = 0);

create table demo.format_probe (id bigserial primary key, name text not null, created_at timestamptz default now(), parent_id bigint references demo.format_probe (id) on delete cascade);
=====
select a, b, c
from demo.orders
where status = 'paid'
  and total > 10
  and customer_id in (1, 2, 3)
order by placed_at desc;

select o.id,
       o.total,
       c.name,
       c.email,
       c.country,
       o.status,
       o.placed_at,
       o.shipped_at,
       o.client_ip,
       o.invoice_no
from demo.orders o
         join demo.customers c on c.id = o.customer_id
         left join demo.order_items i on i.order_id = o.id
where o.total > (select avg(total) from demo.orders where status = 'paid')
group by o.id, c.name
having count(*) > 1;

select id,
       -- own-line comment inside the select list
       total
from demo.orders
where status = 'paid' -- trailing comment
  and total > 10;

select id, case when total > 100 then 'big' when total > 10 then 'mid' else 'small' end as size
from demo.orders;

with recent as (select customer_id, count(*) as n from demo.orders where placed_at > now() - interval '30 days'
group by customer_id), vip as (
select customer_id
from recent
where n > 3)
select c.*
from demo.customers c join vip v
on v.customer_id = c.id;

insert into demo.categories (id, name, parent_id)
values (100, 'probe', null),
       (101, 'probe 2', 100);

update demo.orders
set status     = 'cancelled',
    shipped_at = null
where id in (select order_id from demo.order_items where qty = 0);

create table demo.format_probe
(
    id         bigserial primary key,
    name       text not null,
    created_at timestamptz default now(),
    parent_id  bigint references demo.format_probe (id) on delete cascade
);
