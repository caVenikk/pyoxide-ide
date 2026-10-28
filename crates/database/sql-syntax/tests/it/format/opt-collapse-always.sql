# style: collapse-short-statement=always
select a,
 b from t
where a = 1;
select * from t where a in (select a from u);
=====
select a, b from t where a = 1;
select * from t where a in (select a from u);
