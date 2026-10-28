# dialect: oracle
select level, name from emp start with mgr is null connect by prior id = mgr order by 1;
=====
select level, name
from emp
start with mgr is null connect by prior id = mgr
order by 1;
