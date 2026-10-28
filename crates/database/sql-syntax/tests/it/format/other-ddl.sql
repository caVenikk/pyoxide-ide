create view v as select a from t where a > 1;
drop table if exists t cascade;
create unique index concurrently i on t (lower(a), b desc);
truncate table t;
comment on table t is 'a table';
=====
create view v as
select a
from t
where a > 1;
drop table if exists t cascade;
create unique index concurrently i on t (lower(a), b desc);
truncate table t;
comment on table t is 'a table';
