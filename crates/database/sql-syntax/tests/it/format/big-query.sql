select c.id, c.name, count(o.id) as orders, sum(o.total) total, case when c.vip then 'vip' else 'std' end as kind
from customers c left join orders o on o.customer_id = c.id and o.created > date '2024-01-01'
where c.active = 1 and c.region in ('north', 'south') or c.manager is null
group by c.id, c.name having count(o.id) > 1 order by total desc, c.name limit 10;
=====
select c.id, c.name, count(o.id) as orders, sum(o.total) total, case when c.vip then 'vip' else 'std' end as kind
from customers c
    left join orders o on o.customer_id = c.id and o.created > date '2024-01-01'
where c.active = 1 and c.region in ('north', 'south') or c.manager is null
group by c.id, c.name
having count(o.id) > 1
order by total desc, c.name
limit 10;
