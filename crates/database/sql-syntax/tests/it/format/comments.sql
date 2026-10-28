-- leading
select a, -- first
       b, /* inline */ c
/* before from */
from t -- source
where a = 1 -- one
  and b = 2
/* tail */;
-- last
=====
-- leading
select a, -- first
       b, /* inline */ c
/* before from */
from t -- source
where a = 1 -- one
  and b = 2
/* tail */;
-- last
