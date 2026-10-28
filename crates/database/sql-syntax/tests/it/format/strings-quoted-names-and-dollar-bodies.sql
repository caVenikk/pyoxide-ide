select 'a;b', "Mixed Case", E'x\n;', $q$ ; select $q$ from t;
create function f() returns int language sql as $$ select  1  from   t ; $$;
=====
select 'a;b', "Mixed Case", E'x\n;', $q$ ; select $q$
from t;
create function f() returns int language sql as $$ select  1  from   t ; $$;
