//! Parse benchmarks of ADR 0022 (`docs/performance.md`): generated scripts
//! of 5000 lines (queries; data modification, DDL and PL/pgSQL functions;
//! PL/SQL procedures), parsed whole and re-parsed after a keystroke. Run with
//! `cargo bench -p sql-syntax`; results go to `replica/perf.md`.
use std::hint::black_box;
use std::time::{Duration, Instant};

use sql_syntax::{Dialect, TextEdit, lex, parse};

/// Statements of about ten lines each, varied by `i`.
fn statement(i: usize) -> String {
    match i % 5 {
        0 => format!(
            "select c.id, c.name, c.email,\n       count(o.id) as orders_{i},\n       sum(o.total) as total\n  from customers c\n  left join orders o on o.customer_id = c.id\n where c.active = 1\n   and c.created > date '2024-01-01'\n group by c.id, c.name, c.email\nhaving count(o.id) > {i}\n order by total desc;\n"
        ),
        1 => format!(
            "with recent as (\n  select customer_id, max(created) as last_order\n    from orders\n   where total > {i}\n   group by customer_id\n)\nselect r.customer_id, r.last_order, c.name\n  from recent r\n  join customers c on c.id = r.customer_id\n where r.last_order > current_date - 30;\n"
        ),
        2 => format!(
            "select id,\n       name,\n       row_number() over (partition by region order by revenue desc) as rn,\n       sum(revenue) over (partition by region) as region_total,\n       case when revenue > {i} then 'high'\n            when revenue > 10 then 'mid'\n            else 'low' end as band\n  from sales_{i}\n where region in ('north', 'south', 'east')\n   and revenue between 1 and 1000000;\n"
        ),
        3 => format!(
            "select p.id, p.title\n  from products p\n where exists (\n         select 1\n           from order_items oi\n          where oi.product_id = p.id\n            and oi.quantity > {i}\n       )\n   and p.price > (select avg(price) from products)\n order by p.title;\n"
        ),
        _ => format!(
            "-- report {i}\nselect a.id, b.value, cast(b.amount as numeric(12, 2)) as amount\n  from accounts a\n  join balances b using (account_id)\n where a.kind = 'savings'\n   and b.value is not null\nunion all\nselect a.id, null, 0\n  from accounts a\n where a.closed;\n"
        ),
    }
}

/// Data modification, DDL and PL/pgSQL functions, about ten lines each.
fn mixed_statement(i: usize) -> String {
    match i % 6 {
        0 => format!(
            "insert into orders_{i} (customer_id, total, status, created)\nvalues ({i}, 10.5, 'new', now())\non conflict (customer_id) do update\n   set total = orders_{i}.total + excluded.total,\n       status = excluded.status\n where orders_{i}.status <> 'closed'\nreturning id, total;\n"
        ),
        1 => format!(
            "update accounts a\n   set balance = a.balance - t.total,\n       updated = now()\n  from (select account_id, sum(total) as total\n          from transfers_{i}\n         group by account_id) t\n where a.id = t.account_id\n   and a.balance > {i}\nreturning a.id;\n"
        ),
        2 => format!(
            "create table items_{i} (\n  id bigint generated always as identity primary key,\n  name varchar(100) not null default 'x',\n  total numeric(12, 2) check (total >= 0),\n  parent_id bigint references items_{i} (id) on delete cascade,\n  created timestamp with time zone default now(),\n  constraint uq_{i} unique (name, parent_id)\n) with (fillfactor = 70);\n"
        ),
        3 => format!(
            "create or replace function bump_{i}(p_id int, p_total numeric)\nreturns int language plpgsql as $$\ndeclare\n  v_count int := 0;\nbegin\n  for r in select id, total from items_{i} where parent_id = p_id loop\n    if r.total > p_total then\n      update items_{i} set total = total - 1 where id = r.id;\n      v_count := v_count + 1;\n    end if;\n  end loop;\n  return v_count;\nend\n$$;\n"
        ),
        4 => format!(
            "merge into stock s\nusing (select item_id, sum(total) as total from deliveries_{i} group by item_id) d\n   on s.item_id = d.item_id\n when matched then update set s.total = s.total + d.total\n when not matched then insert (item_id, total) values (d.item_id, d.total);\ndelete from log_{i} where created < now() - interval '30 days';\n"
        ),
        _ => format!(
            "create index concurrently idx_items_{i} on items_{i} (lower(name), total desc) where total > {i};\nalter table items_{i}\n  add column note text,\n  drop column if exists legacy,\n  alter column total set default 0;\ncomment on table items_{i} is 'items';\n"
        ),
    }
}

/// PL/SQL procedures with `/` terminators.
fn plsql_statement(i: usize) -> String {
    format!(
        "create or replace procedure refresh_{i} (p_id in number, p_total out number) is\n  cursor c is select id, total from items_{i} where parent_id = p_id;\n  v_count pls_integer := 0;\nbegin\n  for r in c loop\n    if r.total > {i} then\n      update items_{i} set total = total - 1 where id = r.id;\n      v_count := v_count + 1;\n    elsif r.total is null then\n      delete from items_{i} where id = r.id;\n    end if;\n  end loop;\n  p_total := v_count;\nexception\n  when others then\n    raise;\nend refresh_{i};\n/\n"
    )
}

fn script_of(lines: usize, bytes: usize, make: impl Fn(usize) -> String) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.lines().count() < lines || out.len() < bytes {
        out.push_str(&make(i));
        i += 1;
    }
    out
}

fn script(lines: usize, bytes: usize) -> String {
    script_of(lines, bytes, statement)
}

/// The median and the worst of `runs` timings of `f`.
fn measure(runs: usize, mut f: impl FnMut()) -> (Duration, Duration) {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    (times[runs / 2], times[runs - 1])
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn main() {
    // 5000 lines of these statements are about 140 KB; ADR 0022 assumes
    // about 250 KB for 5000 lines, so that size is measured as well.
    let queries = [Dialect::Postgres, Dialect::Generic];
    for text in [script(5000, 0), script(0, 250 * 1024)] {
        bench(&text, &queries);
    }
    println!("data modification, DDL, PL/pgSQL:");
    bench(&script_of(5000, 0, mixed_statement), &queries);
    println!("PL/SQL:");
    bench(&script_of(5000, 0, plsql_statement), &[Dialect::Oracle]);
}

fn bench(text: &str, dialects: &[Dialect]) {
    let lines = text.lines().count();
    println!(
        "script: {lines} lines, {} KB, {} statements",
        text.len() / 1024,
        text.matches(";\n").count()
    );
    for &dialect in dialects {
        let (lex_med, lex_max) = measure(30, || {
            black_box(lex(black_box(text), dialect));
        });
        let (parse_med, parse_max) = measure(30, || {
            black_box(parse(black_box(text), dialect));
        });
        let parsed = parse(text, dialect);
        assert!(parsed.errors().is_empty(), "{:?}", &parsed.errors()[..1]);

        // A keystroke in the middle of the script: one character typed into
        // a name, then removed again.
        let at = text.len() / 2;
        let at = text[at..].find("total").map_or(at, |i| at + i + 2);
        // The first character boundary at or after the position.
        let at = (at..=text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(at);
        let insert = TextEdit {
            range: at..at,
            text: "x".to_string(),
        };
        let edited = parsed.reparse(&insert);
        let remove = TextEdit {
            range: at..at + 1,
            text: String::new(),
        };
        let (re_med, re_max) = measure(300, || {
            black_box(black_box(&parsed).reparse(black_box(&insert)));
        });
        let (re2_med, _) = measure(300, || {
            black_box(black_box(&edited).reparse(black_box(&remove)));
        });
        let caret = text[at..].find(" from").map_or(at, |i| at + i + 1);
        let (exp_med, exp_max) = measure(300, || {
            black_box(parsed.expected_at(black_box(caret)));
        });
        println!("{dialect:?}:");
        println!(
            "  lex the script          median {} max {}",
            ms(lex_med),
            ms(lex_max)
        );
        println!(
            "  parse the script        median {} max {}",
            ms(parse_med),
            ms(parse_max)
        );
        println!(
            "  reparse after a key     median {} max {} (undo: median {})",
            ms(re_med),
            ms(re_max),
            ms(re2_med)
        );
        println!(
            "  expected_at             median {} max {}",
            ms(exp_med),
            ms(exp_max)
        );
        println!(
            "  tree memory             {:.1} MB",
            parsed.heap_size() as f64 / (1024.0 * 1024.0)
        );
    }
}
