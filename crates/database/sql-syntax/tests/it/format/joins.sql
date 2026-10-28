select * from a join b on a.id = b.id left outer join c on c.id = b.id and c.x > 1 cross join d, e where 1 = 1;
=====
select *
from a
    join b on a.id = b.id
    left outer join c on c.id = b.id and c.x > 1
    cross join d, e
where 1 = 1;
