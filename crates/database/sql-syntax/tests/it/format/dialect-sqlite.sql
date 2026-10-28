# dialect: sqlite
select  a  from t where x glob 'a*' and y is not null limit 1 offset 2;
=====
select a
from t
where x glob 'a*' and y is not null
limit 1
offset 2;
