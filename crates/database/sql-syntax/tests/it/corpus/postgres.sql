-- Valid PostgreSQL queries: the PostgreSQL dialect parses them without an
-- error and sqlparser's PostgreSQL dialect accepts them. Blocks are
-- separated by blank lines; a block that starts with `-- no-oracle` is valid
-- by the manual but not accepted by sqlparser 0.63, and is not given to it.

select id from customers limit 10

select id from customers limit 10 offset 20

select id from customers offset 5

select id from customers order by id limit all

select id from customers order by id offset 10 rows fetch first 5 rows only

select id from customers fetch next 1 row only

select id from customers order by id fetch first 3 rows with ties

select id::text from customers

select '2024-01-01'::date

select total::numeric(10, 2) from orders

select ids::int[] from batches

select now()::timestamp with time zone

select cast(x as double precision) from points

select cast(x as character varying(20)) from points

select cast(created as timestamp without time zone) from orders

select id from customers where name ilike 'a%'

select id from customers where name not ilike '%z'

select id from customers where name similar to '(a|b)%'

select id from customers where name ~ '^A'

select id from customers where name ~* 'a' and name !~ 'z'

select distinct on (customer_id) customer_id, total from orders order by customer_id, total desc

select * from customers c, lateral (select * from orders o where o.customer_id = c.id limit 3) r

select * from customers c left join lateral (select max(total) m from orders o where o.customer_id = c.id) x on true

select * from generate_series(1, 10) as g(n)

select * from generate_series(1, 10) g

select * from unnest(array[1, 2, 3]) with ordinality as u(x, n)

select * from jsonb_to_record('{"a": 1}') as r(a int, b text)

select array[1, 2, 3]

select array[[1, 2], [3, 4]]

select array(select id from customers)

select tags[1], tags[2:3], tags[:2] from posts

select data -> 'a', data ->> 'b', data #> '{a,b}', data #>> '{a}' from docs

select id from docs where data @> '{"a": 1}' and tags && array['x']

select id from docs where data ? 'key'

select row(1, 'a')

select (1, 2) = (1, 2)

select (rec).field from records

select true, false, null

select id from flags where active is true and deleted is not false

select id from flags where state is unknown

select id from customers where email is distinct from phone

select id from customers where email is not distinct from phone

-- no-oracle
select id from customers where email isnull

select id from customers where email notnull

select string_agg(name, ', ' order by name) from customers

select array_agg(id order by id desc) from orders

select count(*) filter (where total > 10) from orders

select percentile_cont(0.5) within group (order by total) from orders

select mode() within group (order by city) from customers

select extract(year from created) from orders

select extract(epoch from now())

select substring(name from 2 for 3) from customers

select substring(name from 2) from customers

select position('a' in name) from customers

select trim(both ' ' from name) from customers

select trim(leading '0' from code) from items

select overlay(name placing 'xx' from 2 for 2) from customers

select date '2024-01-01', timestamp '2024-01-01 10:00', interval '1 day'

select now() at time zone 'UTC'

select name collate "C" from customers order by name collate "C"

select id from customers order by name nulls first

select id from customers order by name desc nulls last

select id from customers order by name using >

select city, count(*) from customers group by rollup (city, country)

select city, count(*) from customers group by cube (city, country)

select city, country, count(*) from customers group by grouping sets ((city), (country), ())

select id, sum(total) over w from orders window w as (partition by customer_id order by id)

select id, sum(total) over (w rows between 1 preceding and current row) from orders window w as (order by id)

-- no-oracle
select id, sum(total) over (order by id groups between 1 preceding and 1 following exclude current row) from orders

select id, avg(total) over (order by created range between interval '7 days' preceding and current row) from orders

-- no-oracle
table customers

select * from only customers

with recursive tree as (select id, parent_id from nodes where parent_id is null union all select n.id, n.parent_id from nodes n join tree t on n.parent_id = t.id) select * from tree

with t as materialized (select 1) select * from t

with t as not materialized (select 1) select * from t

select id from customers for update

select id from customers for update of customers nowait

select id from customers for share skip locked

-- no-oracle
select id from customers for no key update

select $1::int + $2

select id from customers where id = any(array[1, 2])

select id from customers where id = any(select customer_id from orders)

select id from customers where total > all(select total from refunds)

select 'a' || 'b', e'line\nbreak', $$dollar 'quoted'$$, $tag$x$tag$

select current_date, current_timestamp, current_user

select greatest(1, 2), least(3, 4)

select concat_ws(',', a, b) from pairs

select 2 ^ 3, |/ 16, @ -5

-- no-oracle
select x from t where x between symmetric 10 and 1

select id from customers c where not exists (select from orders o where o.customer_id = c.id)

select from customers

select json_build_object('id', id, 'name', name) from customers

select id from customers where (city, country) in (('Paris', 'FR'), ('Rome', 'IT'))

select * from customers c join orders o using (customer_id) where o.total > 1 order by o.total desc limit 5

select now() - interval '1 hour'

select lower(name) from customers group by 1 having count(*) > 1 order by 1 limit 10 offset 10

select * from (values (1), (2)) v(n) order by n desc

select id from customers where name ilike any (array['a%', 'b%'])

select jsonb_path_query(doc, '$.items[*]') from docs

select count(*) from orders where created >= date_trunc('month', now())

select id, lead(total) over (partition by customer_id order by created) from orders

select id, ntile(4) over (order by total) from orders

select id, cume_dist() over (order by total), percent_rank() over (order by total) from orders

select customer_id, bool_and(paid), bool_or(refunded) from orders group by customer_id

select regexp_replace(name, '\s+', ' ', 'g') from customers

select split_part(email, '@', 2) from customers

select to_char(created, 'YYYY-MM') as month, sum(total) from orders group by 1 order by 1

select coalesce(sum(total), 0)::bigint from orders

select id from customers where created between now() - interval '30 days' and now()

select generate_series(1, 3) as n

select c.id from customers c where c.id = any($1::int[])

select o.* from orders o inner join lateral (select 1) x on true where o.total > 0

select a.id, b.id from a full join b on a.key = b.key

select a.id from a right outer join b on a.key = b.key

select id from customers order by random() limit 1

select distinct on (a, b) a, b, c from t order by a, b, c desc

with x as (select 1 as a), y as (select a + 1 as b from x) select x.a, y.b from x cross join y
