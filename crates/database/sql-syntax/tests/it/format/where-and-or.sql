select * from t where a = 1
 and b = 2
 or (c = 3 and d = 4)
 and e in (1, 2, 3);
=====
select *
from t
where a = 1
  and b = 2
   or (c = 3 and d = 4)
  and e in (1, 2, 3);
