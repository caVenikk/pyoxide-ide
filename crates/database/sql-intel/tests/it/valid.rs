//! Valid statements produce no problems: a false unresolved mark is worse
//! than a missed one.
use sql_intel::Problem;
use sql_syntax::Dialect;

use crate::fixture::*;

const COMMON: &[&str] = &[
    "select count(*) from orders",
    "select o.* from orders o",
    "select id from orders group by id having count(*) > 1 order by 1",
    "select case when total > 1 then 'a' end as k from orders order by k",
    "select c.name, (select count(*) from orders where customer_id = c.id) from customers c",
    "select * from orders o where not exists (select 1 from customers c where c.id = o.customer_id)",
    "select t.x from (select 1 as x from orders) t",
    "select orders.id from orders",
    "select id, row_number() over (partition by customer_id order by total) as rn from orders",
    "select o.id, c.name from orders o left join customers c on c.id = o.customer_id where c.id is null",
    "select a.id from orders a join orders b on a.id = b.customer_id",
    "insert into orders (id, customer_id) select id, id from customers",
    "update orders set total = total + 1 where id in (select id from customers)",
    "delete from orders where customer_id not in (select id from customers)",
    "select x.a, x.b from (select id as a, name as b from customers) x where x.a > 0",
    "with t as (select id from orders), u as (select id from t) select u.id from u",
    "select id from orders union select id from customers order by id",
    "select sum(o.total) as s from orders o group by o.customer_id having sum(o.total) > 0",
    "select * from orders where total between 1 and 2 and id in (1, 2)",
    "create table t2 (a int references customers (id)); select a from t2",
    "create index i on orders (total)",
    "alter table orders add constraint fk foreign key (customer_id) references customers (id)",
    "select current_date from orders",
    "select email from app.users u where u.id = 1",
    "select o.id from orders as o",
    "drop table if exists nothing_here",
];

const POSTGRES: &[&str] = &[
    "select * from orders o, lateral (select o.id) x",
    "select total::text from orders",
    "select * from (values (1, 2)) v(a, b) where a = 1",
    "with recursive r(n) as (select 1 union all select n + 1 from r where n < 10) select n from r",
    "select array_agg(id order by total) from orders",
    "select * from orders where id = any(array[1, 2])",
    "insert into orders default values",
    "select coalesce(o.total, 0) from orders o left join customers c using (id)",
    "select distinct on (customer_id) customer_id, total from orders order by customer_id, total desc",
    "select x from unnest(array[1, 2]) as x",
    "select generate_series(1, 3)",
    "select now(), current_timestamp",
    "delete from orders o using customers c where o.customer_id = c.id",
    "select o.id from public.orders o",
    "select public.orders.id from public.orders",
    "select sum(total) over w from orders window w as (partition by customer_id)",
    "select o from orders o",
    "select ctid, xmin from orders",
    "insert into orders (id) values (1) on conflict (id) do nothing",
    "select * from pg_catalog.pg_class c join pg_namespace n on n.oid = c.relnamespace",
    "select table_name from information_schema.tables",
    "create view v as select id, total from orders; select v.total from v",
    "do $$ begin perform 1; end $$",
];

const ORACLE: &[&str] = &[
    "select o.id, rownum from orders o where rownum < 10",
    "select level from dual connect by level < 5",
    "select id from orders start with id = 1 connect by prior id = customer_id",
    "select seq.nextval from dual",
    "select * from orders fetch first 5 rows only",
    "select t.total from orders t where t.id = :id",
    "select sysdate - 1 from dual",
    "select o.id, c.name from orders o, customers c where o.customer_id = c.id(+)",
    "select nvl(total, 0) from orders",
    "merge into orders o using customers c on (o.id = c.id) when matched then update set o.total = 1",
    "begin update orders set total = 0 where id = p_id; end;",
    "declare v number; begin select count(*) into v from orders; end;",
    "select * from user_tables",
];

const SQLITE: &[&str] = &[
    "select * from orders limit 5 offset 1",
    "select rowid, * from orders",
    "insert or replace into orders (id) values (1)",
    "select total as t from orders where t > 1",
    "select id from orders o where o.rowid = 1",
    "create trigger trg after insert on orders begin update customers set name = new.total where id = new.customer_id; end",
    "select name from sqlite_master where type = 'table'",
];

fn check(d: Dialect, text: &str) -> Vec<Problem> {
    problems(text, &scoped(d))
        .into_iter()
        .filter(|p| p.inspection.id() != "SqlWithoutWhere")
        .collect()
}

#[test]
fn valid_statements_have_no_problems() {
    let mut failures = Vec::new();
    for d in RESOLVING {
        let own = match d {
            Dialect::Postgres => POSTGRES,
            Dialect::Oracle => ORACLE,
            _ => SQLITE,
        };
        for text in COMMON.iter().chain(own) {
            let ps = check(d, text);
            if !ps.is_empty() {
                failures.push(format!(
                    "{d:?}: {text}\n    {:?}",
                    ps.iter()
                        .map(|p| format!(
                            "{} {:?} {}",
                            p.inspection,
                            &text[p.range.clone()],
                            p.message
                        ))
                        .collect::<Vec<_>>()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
