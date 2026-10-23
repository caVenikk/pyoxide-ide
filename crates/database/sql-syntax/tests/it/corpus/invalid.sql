-- Invalid queries of the common subset: every dialect reports at least one
-- syntax error, and sqlparser rejects them with its PostgreSQL and SQLite
-- dialects. Blocks are separated by blank lines.

select a, from t

select a from

select a from t where

select * from t join

select (a + b from t

select count(a from t

select * from (select a from t

select * from t where a = and b = 1

select * from t group a

select * from t order by

select a from t union

select case when a then b

select cast(a as) from t

select * from t where a between 1

select a b c from t

select * from t where a = 1)

with as (select 1) select 1

with x as select 1 select 1

select * from t where exists

select a, , b from t

select * from t having

select * from t where a like

select * from t where a is
