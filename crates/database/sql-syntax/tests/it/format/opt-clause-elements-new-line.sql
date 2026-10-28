# style: clause-elements=new-line
select a, b from t join u on t.id = u.id where a = 1 and b = 2 group by a, b order by a;
=====
select
    a, b
from
    t
    join u on t.id = u.id
where
    a = 1 and b = 2
group by
    a, b
order by
    a;
