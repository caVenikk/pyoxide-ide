# style: comma=leading select-items=one-per-line
select a, b, c from t order by a, b;
update t set a = 1,
 b = 2,
 c = 3;
=====
select a
     , b
     , c
from t
order by a, b;
update t
set a = 1
  , b = 2
  , c = 3;
