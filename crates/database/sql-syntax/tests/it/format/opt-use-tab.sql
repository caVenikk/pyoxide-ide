# style: use-tab=true tab-size=4
select * from a join b on a.id = b.id where a.x = 1 and b.y = 2;
create table t (a int, b int);
=====
select *
from a
		 join b on a.id = b.id
where a.x = 1
  and b.y = 2;
create table t
(
	a int,
	b int
);
