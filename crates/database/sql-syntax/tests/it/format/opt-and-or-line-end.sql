# style: and-or=line-end keep-line-breaks=false right-margin=20
select * from t where a = 1 and b = 2 or c = 3;
=====
select *
from t
where a = 1 and
      b = 2 or
      c = 3;
