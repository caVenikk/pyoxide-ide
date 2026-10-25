-- Valid data-modification statements. Blocks are separated by blank lines;
-- the comment lines at the top of a block are directives:
--   dialects: postgres, oracle, sqlite, generic   (default: all four)
--   first: Insert                                  (the kind of the statement node)
--   sqlparser: postgres, sqlite, generic           (the test oracle accepts it too)

-- first: Insert
-- sqlparser: postgres, sqlite, generic
insert into t (a, b) values (1, 2);

-- first: Insert
-- sqlparser: postgres, sqlite, generic
insert into t values (1, 'x'), (2, 'y');

-- first: Insert
-- sqlparser: postgres, sqlite, generic
insert into t select * from s;

-- first: Insert
-- sqlparser: postgres, sqlite, generic
insert into s.t (a) select a from u where x > 1;

-- first: Insert
-- sqlparser: postgres, sqlite, generic
insert into t (a) (select a from u);

-- first: Insert
insert into t (a, b) select a, count(*) from u group by a having count(*) > 1 order by 1;

-- first: Update
-- sqlparser: postgres, sqlite, generic
update t set a = 1;

-- first: Update
-- sqlparser: postgres, sqlite, generic
update t set a = 1, b = b + 1 where id = 5;

-- first: Update
-- sqlparser: postgres, sqlite, generic
update t set a = (select max(x) from u where u.id = t.id) where id in (1, 2, 3);

-- first: Update
-- sqlparser: postgres, sqlite, generic
update s.t set "Name" = 'x' where "Id" = 1;

-- first: Update
update t x set a = 1 where x.id = 2;

-- first: Update
-- sqlparser: postgres, sqlite, generic
update t set a = null, b = case when c > 0 then 1 else 2 end where d between 1 and 2;

-- first: Delete
-- sqlparser: postgres, sqlite, generic
delete from t;

-- first: Delete
-- sqlparser: postgres, sqlite, generic
delete from t where id = 1;

-- first: Delete
-- sqlparser: postgres, sqlite, generic
delete from t where id in (select id from u);

-- first: Delete
-- sqlparser: postgres, sqlite, generic
delete from s.t where exists (select 1 from u where u.id = t.id) and x is null;

-- first: Delete
delete from t x where x.id = 1;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a, b) values (1, 2) returning *;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a) values (1) returning a as x, b + 1;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t default values;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a, b) values (default, 1);

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t as x (a) values (1) on conflict (a) do nothing;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a, b) values (1, 2) on conflict (a) do update set b = excluded.b;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a, b) values (1, 2) on conflict on constraint t_pkey do update set b = excluded.b where t.b < excluded.b returning a;

-- dialects: postgres, generic
-- first: Insert
insert into t (a, b) values (1, 2) on conflict (lower(name), id desc) where id > 0 do nothing;

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a) select a from u on conflict do nothing;

-- dialects: postgres, generic
-- first: Insert
insert into t overriding system value values (1);

-- dialects: postgres, generic
-- first: Insert
-- sqlparser: postgres
insert into t (a, b) values (1, 2) on conflict (a) do update set (b, c) = (1, 2);

-- dialects: postgres, generic
-- first: Insert
with x as (select 1 as a) insert into t select * from x;

-- dialects: postgres, generic
-- first: Insert
with x as (select 1 as a), y as (select 2 as b) insert into t (a, b) select a, b from x, y returning *;

-- dialects: postgres, generic
-- first: Query
-- sqlparser: postgres
with ins as (insert into t (a) values (1) returning id) select * from ins;

-- dialects: postgres, generic
-- first: Insert
with d as (delete from t where id < 10 returning *) insert into archive select * from d;

-- dialects: postgres, generic
-- first: Update
with u as (update t set a = 1 returning id) update o set x = 1 from u where o.id = u.id;

-- dialects: postgres, generic
-- first: Update
-- sqlparser: postgres
update t set a = 1 from u where t.id = u.id;

-- dialects: postgres, generic
-- first: Update
-- sqlparser: postgres
update t set a = u.a from u where u.id = t.id returning t.id, t.a;

-- dialects: postgres, generic
-- first: Update
update only t set a = 1 where current of c;

-- dialects: postgres, generic
-- first: Update
-- sqlparser: postgres
update t set a = default;

-- dialects: postgres, generic
-- first: Update
-- sqlparser: postgres
update t set (a, b) = (1, 2) where id = 3;

-- dialects: postgres, generic
-- first: Update
update t set (a, b) = (select a, b from u where u.id = t.id);

-- dialects: postgres, generic
-- first: Update
update t set arr[1] = 5, arr[2] = 6;

-- dialects: postgres, generic
-- first: Update
update t as x set a = 1 from u as y left join v on v.id = y.id where x.id = y.id;

-- dialects: postgres, generic
-- first: Delete
-- sqlparser: postgres
delete from t using u where t.id = u.id and u.x;

-- dialects: postgres, generic
-- first: Delete
-- sqlparser: postgres
delete from only t where id = 1 returning *;

-- dialects: postgres, generic
-- first: Delete
-- sqlparser: postgres
delete from t using u, v where t.a = u.a returning t.*;

-- dialects: postgres, generic
-- first: Delete
delete from t where current of cur;

-- dialects: postgres, generic
-- first: Merge
-- sqlparser: postgres
merge into t using s on t.id = s.id when matched then update set a = s.a when not matched then insert (id, a) values (s.id, s.a);

-- dialects: postgres, generic
-- first: Merge
-- sqlparser: postgres
merge into t as x using (select 1 as id) as s on x.id = s.id when matched and x.a < 0 then delete when matched then update set a = 0 when not matched then insert values (s.id);

-- dialects: postgres, generic
-- first: Merge
merge into t using s on t.id = s.id when matched then do nothing when not matched by source then delete when not matched by target then insert default values returning t.*;

-- dialects: postgres, generic
-- first: Merge
merge into t using s on t.id = s.id when not matched and s.a > 0 then insert (id) overriding user value values (s.id);

-- dialects: oracle, generic
-- first: Insert
insert into t (a, b) values (1, 2) returning a into :v;

-- dialects: oracle, generic
-- first: Insert
insert into t (a) values (1) returning a, b into v1, v2;

-- dialects: oracle, generic
-- first: Insert
insert into t (a, b) values (1, sysdate) return a into :x;

-- dialects: oracle, generic
-- first: Update
update t set a = 1 where id = 2 returning a into :x;

-- dialects: oracle, generic
-- first: Update
update t set (a, b) = (select 1, 2 from dual) where id = 3;

-- dialects: oracle, generic
-- first: Update
update (select a from t where b > 1) set a = 0;

-- dialects: oracle, generic
-- first: Update
update t set a = (select max(b) from u where u.id = t.id) where exists (select 1 from u where u.id = t.id);

-- dialects: oracle, generic
-- first: Update
update /*+ parallel(t 4) */ t set a = 1 log errors into err$_t ('tag') reject limit 10;

-- dialects: oracle, generic
-- first: Insert
insert all into t1 (a) values (1) into t2 (b) values (2) select * from dual;

-- dialects: oracle, generic
-- first: Insert
insert all when x > 1 then into t1 (a) values (x) into t1b (a) values (x) when x < 0 then into t2 (b) values (x) else into t3 (c) values (x) select x from s;

-- dialects: oracle, generic
-- first: Insert
insert first when a = 1 then into t1 values (a) else into t2 values (a) select a from s;

-- dialects: oracle, generic
-- first: Insert
insert into t partition (p1) values (1);

-- dialects: oracle, generic
-- first: Insert
insert /*+ append */ into t select * from s log errors into e ('tag') reject limit unlimited;

-- dialects: oracle, generic
-- first: Merge
merge into t using s on (t.id = s.id) when matched then update set t.a = s.a where s.a > 0 delete where t.b = 1 when not matched then insert (id, a) values (s.id, s.a) where s.a > 0;

-- dialects: oracle, generic
-- first: Merge
merge into t x using (select 1 id from dual) s on (x.id = s.id) when matched then update set x.a = 1;

-- dialects: oracle, generic
-- first: Merge
merge into t using dual on (t.id = 1) when not matched then insert (id) values (1);

-- dialects: oracle, generic
-- first: Merge
merge into t using s on (t.id = s.id) when not matched then insert (id) values (s.id) when matched then update set t.a = s.a;

-- dialects: oracle, generic
-- first: Delete
delete t where id = 1;

-- dialects: oracle, generic
-- first: Delete
delete from t t1 where t1.id = 5 returning id into :x;

-- dialects: oracle, generic
-- first: Delete
delete t;

-- dialects: oracle, generic
-- first: Delete
delete from (select * from t where a > 1) where b = 2;

-- dialects: oracle, generic
-- first: Delete
delete from t partition (p1) where a = 1;

-- dialects: sqlite, generic
-- first: Insert
-- sqlparser: sqlite
insert or replace into t (a) values (1);

-- dialects: sqlite, generic
-- first: Insert
-- sqlparser: sqlite
insert or ignore into t values (1, 2);

-- dialects: sqlite, generic
-- first: Insert
-- sqlparser: sqlite
replace into t (a) values (1);

-- dialects: sqlite, generic
-- first: Insert
-- sqlparser: sqlite
insert into t (a) values (1) on conflict (a) do update set a = excluded.a;

-- dialects: sqlite, generic
-- first: Insert
-- sqlparser: sqlite
insert into t (a) values (1) on conflict do nothing returning a;

-- dialects: sqlite, generic
-- first: Insert
insert into t (a) select a from u where true on conflict (a) do update set a = 1 where excluded.a > t.a;

-- dialects: sqlite, generic
-- first: Insert
insert into t default values;

-- dialects: sqlite, generic
-- first: Update
-- sqlparser: sqlite
update or fail t set a = 1;

-- dialects: sqlite, generic
-- first: Update
-- sqlparser: sqlite
update t set a = 1 from u where t.id = u.id;

-- dialects: sqlite, generic
-- first: Update
update t set a = 1 returning *;

-- dialects: sqlite, generic
-- first: Update
update t set a = 1 order by id limit 3;

-- dialects: sqlite, generic
-- first: Update
update t indexed by i set a = 1;

-- dialects: sqlite, generic
-- first: Delete
-- sqlparser: sqlite
delete from t where id = 1 returning id;

-- dialects: sqlite, generic
-- first: Delete
delete from t order by id limit 1;

-- dialects: sqlite, generic
-- first: Delete
with c as (select 1) delete from t where id in (select * from c);
