# style: and-or=line-end
select * from t where a = 1 and
 b = 2 and
 c = 3;
=====
select *
from t
where a = 1 and
      b = 2 and
      c = 3;
