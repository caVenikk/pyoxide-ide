-- Valid PostgreSQL routines: DO blocks, function and procedure bodies in
-- dollar quotes (parsed as embedded fragments), triggers. Blocks have no
-- blank lines. The default dialects are PostgreSQL and generic SQL.

-- first: DoStatement
do $$ begin raise notice 'x'; end $$;

-- first: DoStatement
do $$
declare
  n int;
begin
  select count(*) into n from t;
  raise notice 'rows: %', n;
end
$$;

-- first: DoStatement
do language plpgsql $body$
begin
  perform pg_sleep(1);
end
$body$;

-- first: DoStatement
do $$ begin null; end $$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns int language plpgsql as $$
begin
  return a + 1;
end
$$;

-- first: CreateFunction
create or replace function s.f(a int, b text default 'x', out c int) returns int as $$
declare
  x int := 0;
  y constant text := 'y';
  r record;
  arr int[];
begin
  x := a * 2;
  x = x + 1;
  c := x;
  return x;
end;
$$ language plpgsql immutable strict;

-- first: CreateFunction
create function f(a int) returns text as $fn$
begin
  if a > 10 then
    return 'big';
  elsif a > 5 then
    return 'mid';
  elseif a > 1 then
    return 'small';
  else
    return 'tiny';
  end if;
end
$fn$ language plpgsql;

-- first: CreateFunction
create function f(n int) returns int as $$
declare
  total int := 0;
begin
  for i in 1..n loop
    total := total + i;
  end loop;
  for i in reverse 10..1 by 2 loop
    total := total - 1;
  end loop;
  while total > 100 loop
    total := total / 2;
    exit when total < 10;
  end loop;
  loop
    total := total + 1;
    exit;
  end loop;
  return total;
end
$$ language plpgsql;

-- first: CreateFunction
create function f() returns setof t as $$
declare
  r t%rowtype;
begin
  for r in select * from t where id > 1 loop
    return next r;
  end loop;
  for r in execute 'select * from ' || quote_ident('t') loop
    return next r;
  end loop;
  return query select * from t;
  return query execute 'select 1' using 5;
  return;
end
$$ language plpgsql;

-- first: CreateFunction
create function f() returns trigger as $$
begin
  new.updated := now();
  if tg_op = 'INSERT' then
    new.created := now();
  elsif tg_op = 'DELETE' then
    return old;
  end if;
  return new;
end
$$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns int as $$
begin
  perform 1;
  perform g(a), h(a) from t where x = a;
  insert into t (a) values (a) returning id into new_id;
  update t set a = a + 1 where id = 1;
  delete from t where id = 2;
  select a, b into x, y from t where id = 1;
  select a into strict z from t where id = 1;
  execute 'update t set a = 1';
  execute format('select %I from %I', c, t) into v using a, b;
  call p(1);
  return 1;
end
$$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns void as $$
begin
  raise exception 'bad value %', a using errcode = 'P0001', hint = 'fix it';
  raise notice 'x';
  raise warning 'w % %', a, a;
  raise sqlstate '22012';
  raise division_by_zero;
  raise;
end
$$ language plpgsql;

-- first: CreateFunction
create function f() returns int as $$
begin
  return 1;
exception
  when division_by_zero then
    return 0;
  when unique_violation or foreign_key_violation then
    raise notice 'dup';
  when sqlstate '23505' then
    null;
  when others then
    get stacked diagnostics msg = message_text;
    raise;
end
$$ language plpgsql;

-- first: CreateFunction
create function f() returns void as $$
declare
  c1 cursor for select * from t;
  c2 cursor (p int) for select * from t where id = p;
  c3 refcursor;
  r t%rowtype;
begin
  open c1;
  fetch c1 into r;
  fetch next from c1 into r;
  move forward 2 from c1;
  close c1;
  open c2(5);
  open c3 for select * from t;
  open c3 for execute 'select 1';
  foreach x in array arr loop
    null;
  end loop;
  foreach y slice 1 in array arr2 loop
    null;
  end loop;
end
$$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns int as $$
begin
  case a
    when 1, 2 then return 10;
    when 3 then return 30;
    else return 0;
  end case;
end
$$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns int as $$
begin
  case
    when a > 1 then return 1;
    else return 2;
  end case;
end
$$ language plpgsql;

-- first: CreateFunction
create function f() returns void as $$
<<outer>>
declare
  x int;
begin
  <<inner>>
  for i in 1..3 loop
    exit outer when i > 2;
    continue inner;
  end loop;
  declare
    y int := 1;
  begin
    x := y;
  end;
end outer
$$ language plpgsql;

-- first: CreateFunction
create function f() returns void as $$
begin
  assert 1 = 1, 'math broke';
  commit;
  rollback;
  get diagnostics n = row_count;
  set local x = 1;
  lock table t in share mode;
  notify ch, 'x';
end
$$ language plpgsql;

-- first: CreateFunction
create function f(a int) returns int language sql as $$ select a + 1 $$;

-- first: CreateFunction
create function f() returns setof t language sql as $$
  select * from t where id > 1;
$$;

-- first: CreateFunction
create function f(a int, b int) returns int as $$
  insert into t (a, b) values ($1, $2);
  select 1;
$$ language sql volatile;

-- first: CreateFunction
create function f(a int) returns int language sql immutable parallel safe return a + 1;

-- first: CreateFunction
create function f(a int) returns int language sql begin atomic select a + 1; end;

-- first: CreateFunction
create function f(a int) returns int as 'select a + 1' language sql;

-- first: CreateFunction
create function f() returns void as 'begin null; end;' language plpgsql;

-- first: CreateFunction
create function f(a int) returns int as 'MODULE_PATHNAME', 'f_c' language c strict;

-- first: CreateFunction
create function f() returns int as $$ print("hi") $$ language plpython3u;

-- first: CreateFunction
create function f(a int) returns table (x int, y text) as $$
begin
  return query select 1, 'a';
end
$$ language plpgsql security definer set search_path = public, pg_temp cost 100 rows 10;

-- first: CreateFunction
create function f(variadic a int[]) returns int as $$ select 1 $$ language sql;

-- first: CreateFunction
create function f(in a int, inout b int, out c int, d text default null) returns record as $$ select 1, 2 $$ language sql;

-- first: CreateFunction
create function agg_sfunc(state int, v int) returns int language sql immutable as 'select $1 + $2';

-- first: CreateProcedure
create procedure p(a int) language plpgsql as $$
begin
  insert into t values (a);
  commit;
end
$$;

-- first: CreateProcedure
create or replace procedure s.p() language sql as $$ delete from t $$;

-- first: CreateProcedure
create procedure p(a int) language sql begin atomic insert into t values (a); delete from u; end;

-- first: CreateTrigger
create trigger tr before insert or update on t for each row execute function f();

-- first: CreateTrigger
create trigger tr after update of a, b on s.t for each row when (old.a is distinct from new.a) execute function f('x', 1);

-- first: CreateTrigger
create or replace trigger tr after insert on t referencing new table as nt for each statement execute procedure f();

-- first: CreateTrigger
create constraint trigger tr after insert on t deferrable initially deferred for each row execute function f();

-- first: CreateTrigger
create trigger tr instead of delete on v for each row execute function f();

-- first: CreateTrigger
create trigger tr before truncate on t for each statement execute function f();
