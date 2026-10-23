-- Valid queries of the common subset: every dialect parses them without an
-- error, and sqlparser accepts them with its generic, PostgreSQL and SQLite
-- dialects. Blocks are separated by blank lines.

select 1

select * from customers

select id, name from customers

select c.id, c.name from customers c

select customers.id from customers

select s.customers.id from s.customers

select c.* from customers c

select *, id from customers

select id as customer_id, name customer_name from customers

select "Id", "Order Items"."Total" from "Order Items"

select distinct city from customers

select all city from customers

select count(*) from orders

select count(distinct customer_id) from orders

select sum(total), avg(total), min(total), max(total) from orders

select upper(name), lower(email), length(name) from customers

select coalesce(phone, email, 'none') from customers

select nullif(total, 0) from orders

select abs(-5), round(total, 2) from orders

select id from customers where id = 1

select id from customers where id <> 1 and name is not null

select id from customers where id != 2 or id < 10

select id from customers where not (id > 5)

select id from customers where id between 1 and 10

select id from customers where id not between 1 and 10

select id from customers where id in (1, 2, 3)

select id from customers where id not in (1, 2)

select id from customers where name like 'A%'

select id from customers where name not like '%x%'

select id from customers where name like 'a!%' escape '!'

select id from customers where email is null

select id from customers where email is not null

select id from orders where total >= 10.5 and total <= 99.99

select id from orders where total > 1e3

select id from customers where (city = 'Paris' or city = 'Rome') and active = 1

select id, total * 2 + 1, (total - 1) / 3, total % 7 from orders

select name || ' <' || email || '>' from customers

select -total, +total from orders

select case when total > 100 then 'big' when total > 10 then 'medium' else 'small' end from orders

select case status when 1 then 'new' when 2 then 'paid' end from orders

select case when total is null then 0 else total end as t from orders

select cast(total as integer) from orders

select cast(id as varchar(10)) from customers

select cast(created as date) from orders

select id from customers where exists (select 1 from orders o where o.customer_id = customers.id)

select id from customers where not exists (select 1 from orders where orders.customer_id = customers.id)

select id from customers where id in (select customer_id from orders)

select id from customers where id not in (select customer_id from orders where total > 5)

select (select count(*) from orders o where o.customer_id = c.id) as n from customers c

select id from orders where total > (select avg(total) from orders)

select * from (select id, name from customers) sub

select sub.id from (select id from customers where active = 1) as sub

select * from customers c join orders o on o.customer_id = c.id

select * from customers c inner join orders o on o.customer_id = c.id

select * from customers c left join orders o on o.customer_id = c.id

select * from customers c left outer join orders o on o.customer_id = c.id

select * from customers c cross join regions r

select * from customers natural join orders

select * from customers join orders using (customer_id)

select * from customers c join orders o on o.customer_id = c.id join items i on i.order_id = o.id

select * from customers c left join orders o on o.customer_id = c.id and o.total > 0

select * from customers, orders where orders.customer_id = customers.id

select * from a, b, c where a.id = b.a_id and b.id = c.b_id

select * from (a join b on a.id = b.a_id) join c on c.b_id = b.id

select city, count(*) from customers group by city

select city, count(*) from customers group by city having count(*) > 1

select city, country, count(*) from customers group by city, country order by country, city

select id from customers order by name

select id from customers order by name desc, id asc

select id from customers order by 1

select city from customers union select city from suppliers

select city from customers union all select city from suppliers

select city from customers intersect select city from suppliers

select city from customers except select city from suppliers

select city from customers union select city from suppliers order by city

select a from t1 union select a from t2 union select a from t3

select a from t1 union select a from t2 intersect select a from t3

with recent as (select * from orders where total > 10) select * from recent

with a as (select 1 as x), b as (select x + 1 as y from a) select * from b

with recursive n (i) as (select 1 union all select i + 1 from n where i < 10) select i from n

with totals (customer_id, total) as (select customer_id, sum(total) from orders group by customer_id) select * from totals

select id, row_number() over (order by total desc) from orders

select id, rank() over (partition by customer_id order by total) from orders

select id, sum(total) over (partition by customer_id) from orders

select id, sum(total) over (order by id rows between unbounded preceding and current row) from orders

select id, avg(total) over (order by id rows between 2 preceding and 2 following) from orders

select id, lag(total, 1) over (order by created) from orders

select id, first_value(total) over (partition by customer_id order by created range between unbounded preceding and unbounded following) from orders

select id, count(*) over () from orders

select values_a.x from (values (1, 'a'), (2, 'b')) as values_a (x, y)

values (1, 'one'), (2, 'two')

select id from customers where name = :name

select 'it''s' from customers

select id from customers -- trailing comment

select /* inline */ id from customers

select id
from customers
where active = 1
order by id

select c.name, o.total
  from customers c
  join orders o on o.customer_id = c.id
 where o.total > 100
   and c.active = 1
 order by o.total desc

select count(*) filtered_count from orders where total > 0

select max(total) - min(total) as spread from orders group by customer_id having max(total) > 10

select o.id from orders o where o.customer_id in (select c.id from customers c where c.city in ('Paris', 'Rome'))

select case when exists (select 1 from orders) then 1 else 0 end from customers

select id from customers where coalesce(phone, '') = ''

select trim(name) from customers

select substr(name, 1, 3) from customers

select replace(name, 'a', 'b') from customers

select id from customers where name like 'J%' and (city = 'Oslo' or city is null)

select t.id from (select id from customers union select id from suppliers) t

select * from customers where id = (select max(id) from customers)

select c.id, (select max(o.total) from orders o where o.customer_id = c.id) from customers c where c.id > 0
