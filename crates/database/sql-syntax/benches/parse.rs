//! Parse benchmarks of ADR 0022 (`docs/performance.md`): a generated script
//! of 5000 lines, parsed whole and re-parsed after a keystroke. Run with
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

fn script(lines: usize, bytes: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.lines().count() < lines || out.len() < bytes {
        out.push_str(&statement(i));
        i += 1;
    }
    out
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
    for text in [script(5000, 0), script(0, 250 * 1024)] {
        bench(&text);
    }
}

fn bench(text: &str) {
    let lines = text.lines().count();
    println!(
        "script: {lines} lines, {} KB, {} statements",
        text.len() / 1024,
        text.matches(";\n").count()
    );
    for dialect in [Dialect::Postgres, Dialect::Generic] {
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
