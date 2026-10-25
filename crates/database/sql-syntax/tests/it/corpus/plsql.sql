-- Valid Oracle PL/SQL: each block is one statement (a block, or a CREATE of a
-- routine, trigger, package or type body). A block has no blank lines.

-- first: Block
begin null; end;

-- first: Block
begin
  dbms_output.put_line('hello');
end;

-- first: Block
declare
  v number := 1;
begin
  v := v + 1;
  dbms_output.put_line(v);
end;

-- first: Block
declare
  v_name varchar2(100);
  v_id constant number := 5;
  c_max constant pls_integer := 10;
  v_row t%rowtype;
  v_col t.c%type;
  v_flag boolean default false;
  v_big number(12, 2) not null := 0;
begin
  select name into v_name from t where id = v_id;
  select * into v_row from t where id = 1;
  select count(*), max(c) into v_id, v_col from t;
end;

-- first: Block
begin
  if x > 1 then
    y := 1;
  elsif x = 1 then
    y := 2;
  else
    y := 3;
  end if;
end;

-- first: Block
begin
  if a and b or not c then null; end if;
  if x is null then null; end if;
  if x between 1 and 2 then null; end if;
end;

-- first: Block
begin
  case x
    when 1 then y := 'a';
    when 2 then y := 'b';
    else y := 'c';
  end case;
  case
    when x > 1 then y := 1;
    when x < 0 then y := 2;
  end case;
end;

-- first: Block
declare
  x number;
begin
  x := case when y > 0 then 1 else 2 end;
  x := nvl(y, 0) + 1;
  s := 'a' || 'b' || to_char(sysdate, 'YYYY');
end;

-- first: Block
begin
  loop
    i := i + 1;
    exit when i > 10;
  end loop;
  while i > 0 loop
    i := i - 1;
    continue when i = 5;
  end loop;
  for i in 1..10 loop
    null;
  end loop;
  for i in reverse 1..n loop
    null;
  end loop;
end;

-- first: Block
begin
  for r in (select id, name from t where id > 1) loop
    dbms_output.put_line(r.id || r.name);
  end loop;
  for r in c(1, 2) loop
    null;
  end loop;
end;

-- first: Block
declare
  cursor c is select id, name from t;
  cursor c2 (p_id number, p_name varchar2 := 'x') is select * from t where id = p_id;
  cursor c3 return t%rowtype;
  r c%rowtype;
begin
  open c;
  loop
    fetch c into r;
    exit when c%notfound;
    dbms_output.put_line(c%rowcount);
  end loop;
  close c;
  open c2(1);
  if c2%isopen then close c2; end if;
end;

-- first: Block
declare
  type t_ids is table of number index by pls_integer;
  type t_rec is record (id number, name varchar2(10));
  type t_cur is ref cursor;
  subtype small is number(3);
  ids t_ids;
  rc t_cur;
begin
  select id bulk collect into ids from t;
  forall i in 1..ids.count
    update t set a = 1 where id = ids(i);
  forall i in indices of ids insert into u values (ids(i));
  open rc for select * from t where id = :id;
  open rc for 'select * from t where id = :1' using 5;
end;

-- first: Block
declare
  c sys_refcursor;
  r t%rowtype;
begin
  fetch c bulk collect into ids limit 100;
  fetch c into r.id, r.name;
  execute immediate 'truncate table t';
  execute immediate 'select count(*) from t where a = :1' into n using in v;
  execute immediate 'begin p(:1, :2); end;' using in a, out b;
  execute immediate 'insert into t values (:1) returning id into :2' using x returning into y;
end;

-- first: Block
begin
  insert into t (a, b) values (1, 2);
  update t set a = 1 where id = 2;
  delete from t where id = 3;
  merge into t using s on (t.id = s.id) when matched then update set t.a = s.a;
  insert into t (a) values (1) returning id into v_id;
  update t set a = a + 1 returning a into v_a;
  delete from t where id = 1 returning name bulk collect into names;
  commit;
  rollback;
  savepoint sp;
  rollback to sp;
end;

-- first: Block
begin
  select a into v from t where id = 1;
exception
  when no_data_found then
    v := 0;
  when too_many_rows or dup_val_on_index then
    raise;
  when others then
    dbms_output.put_line(sqlerrm);
    raise_application_error(-20001, 'failed: ' || sqlerrm);
end;

-- first: Block
declare
  e_custom exception;
  pragma exception_init(e_custom, -20100);
begin
  raise e_custom;
exception
  when e_custom then null;
end;

-- first: Block
<<outer>>
declare
  x number;
begin
  <<inner>>
  for i in 1..3 loop
    for j in 1..3 loop
      exit outer when i * j > 4;
      continue inner when j = 2;
    end loop;
  end loop;
end outer;

-- first: Block
declare
  x number := 1;
begin
  declare
    y number := x + 1;
  begin
    begin
      null;
    exception
      when others then null;
    end;
  end;
end;

-- first: Block
declare
  procedure log(msg varchar2) is
  begin
    insert into logs values (msg);
  end;
  function twice(n number) return number is
  begin
    return n * 2;
  end twice;
  function fwd(n number) return number;
begin
  log('x');
  x := twice(2);
  goto done;
  <<done>>
  null;
end;

-- first: Block
declare
  pragma autonomous_transaction;
begin
  insert into t values (1);
  commit;
end;

-- first: Block
begin
  pkg.proc(1, 'a');
  pkg.proc(p_a => 1, p_b => 'x');
  obj.method;
  x := pkg.func(1) + pkg.other;
  coll(1).field := 5;
  rec.f := rec.g + 1;
  arr(i) := arr(i - 1) * 2;
end;

-- first: Block
begin
  :new_var := 1;
  x := sql%rowcount;
  if sql%notfound then null; end if;
  dbms_output.put_line(sql%bulk_rowcount(1));
end;

-- first: Block
declare
  v_ts timestamp := systimestamp;
  v_iv interval day to second := interval '1' day;
  v_d date := to_date('2024-01-01', 'YYYY-MM-DD');
  v_n number;
begin
  v_n := trunc(sysdate) - v_d;
  v_n := extract(year from sysdate);
  select sysdate - 1 into v_d from dual;
end;

-- first: CreateProcedure
create or replace procedure p is
begin
  null;
end;

-- first: CreateProcedure
create procedure p (a in number, b out varchar2, c in out nocopy number default 1) as
  v number;
begin
  b := to_char(a);
  c := c + a;
end p;

-- first: CreateProcedure
create or replace editionable procedure s.p authid current_user is
begin
  null;
end;

-- first: CreateFunction
create or replace function f (n number) return number is
begin
  return n + 1;
end;

-- first: CreateFunction
create or replace function f (n in number, s in varchar2 default 'x') return varchar2 deterministic is
  r varchar2(100);
begin
  r := s || n;
  return r;
exception
  when others then return null;
end f;

-- first: CreateFunction
create function f return t_tab pipelined is
begin
  for r in (select * from t) loop
    pipe row (r);
  end loop;
  return;
end;

-- first: CreateFunction
create or replace function f(p_id t.id%type) return t%rowtype result_cache is
  r t%rowtype;
begin
  select * into r from t where id = p_id;
  return r;
end;

-- first: CreateFunction
create or replace function jf (a number) return number as language java name 'Foo.bar(int) return int';

-- first: CreatePackage
create or replace package pkg is
  c_max constant number := 10;
  type t_rec is record (a number, b varchar2(10));
  cursor c is select * from t;
  e_err exception;
  procedure p (a number);
  function f (a number) return number;
end pkg;

-- first: CreatePackage
create package pkg authid definer as
  g_state number;
  procedure init;
end;

-- first: CreatePackageBody
create or replace package body pkg is
  g_count number := 0;
  procedure p (a number) is
  begin
    g_count := g_count + a;
  end p;
  function f (a number) return number is
  begin
    return a * 2;
  end f;
  procedure private_one is
  begin
    null;
  end;
begin
  g_count := 1;
end pkg;

-- first: CreatePackageBody
create or replace package body pkg as
  procedure p is
  begin
    null;
  exception
    when others then raise;
  end;
end;

-- first: CreateTrigger
create or replace trigger tr before insert or update on t for each row
begin
  :new.updated := sysdate;
  if :old.status <> :new.status then
    insert into audit_log values (:old.id, :old.status, :new.status);
  end if;
end;

-- first: CreateTrigger
create trigger tr after delete on t
declare
  n number;
begin
  select count(*) into n from t;
end;

-- first: CreateTrigger
create or replace trigger tr before insert on t for each row when (new.id is null)
begin
  :new.id := seq.nextval;
end;

-- first: CreateTrigger
create trigger tr instead of insert on v for each row
begin
  insert into t values (:new.a);
end;

-- first: CreateTrigger
create or replace trigger tr after update of a, b on s.t referencing new as n old as o for each row follows other_tr
begin
  null;
end;

-- first: CreateTrigger
create or replace trigger tr after logon on database
begin
  null;
end;

-- first: CreateTypeBody
create or replace type body t_obj as
  member function f return number is
  begin
    return id * 2;
  end;
  static procedure p is
  begin
    null;
  end;
end;
