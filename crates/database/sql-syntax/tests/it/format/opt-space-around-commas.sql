# style: space-before-comma=true space-after-comma=false
select f(a,b), c from t;
=====
select f(a ,b) ,c
from t;
