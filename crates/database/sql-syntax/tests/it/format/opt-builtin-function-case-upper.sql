# style: builtin-function-case=upper
select Count(*), lower(a), MyFunc(a) from t;
=====
select COUNT(*), LOWER(a), MyFunc(a)
from t;
