# dialect: oracle
select q'[it's; fine]' as s, nq'{x}' from dual where a = :b;
=====
select q'[it's; fine]' as s, nq'{x}'
from dual
where a = :b;
