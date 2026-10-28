# style: keep-line-breaks=false break-conditions=false
select a,
 b
from t
where a = 1
 and b = 2
 and c = 3;
select case when a then 1
 else 2 end from t;
=====
select a, b
from t
where a = 1 and b = 2 and c = 3;
select case when a then 1 else 2 end
from t;
