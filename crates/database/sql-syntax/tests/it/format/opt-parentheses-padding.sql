# style: parentheses-padding=true
select f(a, b), (a + b), (select 1) from t where a in (1, 2);
=====
select f( a, b ), ( a + b ), ( select 1 )
from t
where a in ( 1, 2 );
