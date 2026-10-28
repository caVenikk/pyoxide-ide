# style: spaces-around-operators=false
select a+b, a = 1, c||d from t where x>=1;
=====
select a+b, a=1, c||d
from t
where x>=1;
