select case when a > 1 then 'x' when a > 2 then 'y' else 'z' end,
 case
 when b then 1 else 2 end from t;
=====
select case when a > 1 then 'x' when a > 2 then 'y' else 'z' end,
       case
           when b then 1
           else 2 end
from t;
