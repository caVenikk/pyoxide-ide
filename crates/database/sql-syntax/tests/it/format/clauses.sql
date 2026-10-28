select a, b from t where a = 1 group by a, b having count(*) > 1 order by a limit 5;
=====
select a, b
from t
where a = 1
group by a, b
having count(*) > 1
order by a
limit 5;
