# style: break-conditions=false
select * from t where a = 1 and b = 2
 and c = 3 or d = 4;
=====
select *
from t
where a = 1 and b = 2
    and c = 3 or d = 4;
