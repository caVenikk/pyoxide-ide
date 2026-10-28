# style: collapse-short-statement=never
select * from t where a in (select a from u) and exists (select 1 from v);
with c as (select 1) select * from c;
=====
select *
from t
where a in (select a
            from u)
  and exists (select 1
              from v);
with c as (select 1)
select *
from c;
