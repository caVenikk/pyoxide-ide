select (select max(x) from u) m, a from t where a in (select a from v where v.k = t.k) and exists (select 1 from w);
=====
select (select max(x) from u) m, a
from t
where a in (select a from v where v.k = t.k)
  and exists (select 1 from w);
