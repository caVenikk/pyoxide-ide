select a from t union all select b from u except select c from v order by 1;
=====
select a
from t
union all
select b
from u
except
select c
from v
order by 1;
