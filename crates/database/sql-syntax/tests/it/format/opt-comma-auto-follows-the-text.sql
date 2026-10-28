# style: comma=auto
select a
     , b
     , c
from t order by a, b;
=====
select a
     , b
     , c
from t
order by a, b;
