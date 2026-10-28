# style: space-before-parenthesis=false
insert into t (a, b) values (1, 2);
select * from t where a in (1, 2);
=====
insert into t(a, b)
values(1, 2);
select *
from t
where a in(1, 2);
