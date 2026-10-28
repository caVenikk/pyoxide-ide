with recent as (select customer_id, max(created) as last_order from orders group by customer_id), big as (select * from big_table_with_a_rather_long_name where total > 1000000 and status = 'open' and region = 'north') select * from recent join big using (customer_id);
=====
with recent as (select customer_id, max(created) as last_order from orders group by customer_id),
     big as (select *
             from big_table_with_a_rather_long_name
             where total > 1000000
               and status = 'open'
               and region = 'north')
select *
from recent
         join big using (customer_id);
