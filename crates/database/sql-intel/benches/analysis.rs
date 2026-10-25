//! Diagnostics benchmarks of ADR 0022 (`docs/performance.md`): a generated
//! 5000-line script bound to a model of 10 000 tables, analysed on open
//! (resolution and inspections of every statement) and after a keystroke
//! (re-parse and analysis of the edited statement, the others from the
//! cache). Run with `cargo bench -p sql-intel`; results go to
//! `replica/perf.md`.
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::DataSourceId;
use db_core::schema::{
    ColumnInfo, Level, ObjectInfo, ObjectKind, RelationInfo, SchemaInfo, SchemaSnapshot, Snapshot,
};
use db_model::Model;
use sql_intel::{Analyzer, InspectionProfile, Scope, SqlContext};
use sql_syntax::{Dialect, TextEdit, parse};

const TABLES: usize = 10_000;

fn table(name: &str, columns: &[&str]) -> (ObjectInfo, RelationInfo) {
    (
        ObjectInfo {
            schema: "public".into(),
            name: name.into(),
            kind: ObjectKind::Table,
            ..ObjectInfo::default()
        },
        RelationInfo {
            name: name.into(),
            columns: columns
                .iter()
                .enumerate()
                .map(|(i, c)| ColumnInfo {
                    name: c.to_string(),
                    position: i as u32 + 1,
                    data_type: "integer".into(),
                    nullable: true,
                    ..ColumnInfo::default()
                })
                .collect(),
            indexes: Vec::new(),
            constraints: Vec::new(),
        },
    )
}

/// `public` with the tables the statements use and `TABLES` more of eight
/// columns.
fn model() -> Arc<Model> {
    let mut objects = Vec::new();
    let mut relations = Vec::new();
    let mut add = |(o, r): (ObjectInfo, RelationInfo)| {
        objects.push(o);
        relations.push(r);
    };
    add(table(
        "customers",
        &["id", "name", "email", "active", "created"],
    ));
    add(table(
        "orders",
        &["id", "customer_id", "total", "created", "status"],
    ));
    add(table("products", &["id", "title", "price"]));
    add(table("order_items", &["id", "product_id", "quantity"]));
    add(table("accounts", &["id", "account_id", "kind", "closed"]));
    add(table("balances", &["account_id", "value", "amount"]));
    for t in 0..TABLES {
        add(table(
            &format!("table_{t:05}"),
            &[
                "id",
                "parent_id",
                "name",
                "region",
                "revenue",
                "status",
                "created",
                "updated",
            ],
        ));
    }
    let snapshot = Snapshot {
        level: Level::Structure,
        token: None,
        schemas: vec![SchemaSnapshot {
            info: SchemaInfo {
                name: "public".into(),
                is_current: true,
                ..SchemaInfo::default()
            },
            level: Level::Structure,
            objects,
            relations,
            sources: Vec::new(),
        }],
    };
    Arc::new(Model::from_snapshot(DataSourceId::new("bench"), snapshot))
}

/// Statements of about ten lines each, varied by `i`, over the model's
/// tables.
fn statement(i: usize) -> String {
    let t = i % TABLES;
    match i % 5 {
        0 => format!(
            "select c.id, c.name, c.email,\n       count(o.id) as orders_{i},\n       sum(o.total) as total\n  from customers c\n  left join orders o on o.customer_id = c.id\n where c.active = 1\n   and c.created > date '2024-01-01'\n group by c.id, c.name, c.email\nhaving count(o.id) > {i}\n order by total desc;\n"
        ),
        1 => format!(
            "with recent as (\n  select customer_id, max(created) as last_order\n    from orders\n   where total > {i}\n   group by customer_id\n)\nselect r.customer_id, r.last_order, c.name\n  from recent r\n  join customers c on c.id = r.customer_id\n where r.last_order > current_date - 30;\n"
        ),
        2 => format!(
            "select id,\n       name,\n       row_number() over (partition by region order by revenue desc) as rn,\n       sum(revenue) over (partition by region) as region_total,\n       case when revenue > {i} then 'high'\n            when revenue > 10 then 'mid'\n            else 'low' end as band\n  from table_{t:05}\n where region in ('north', 'south', 'east')\n   and revenue between 1 and 1000000;\n"
        ),
        3 => format!(
            "select p.id, p.title\n  from products p\n where exists (\n         select 1\n           from order_items oi\n          where oi.product_id = p.id\n            and oi.quantity > {i}\n       )\n   and p.price > (select avg(price) from products)\n order by p.title;\n"
        ),
        _ => format!(
            "-- report {i}\nupdate table_{t:05} t\n   set status = 'done', updated = now()\n  from accounts a\n where a.id = t.parent_id\n   and a.kind = 'savings'\n   and t.revenue is not null;\ndelete from table_{t:05}\n where created < current_date - 30\n   and status = 'done';\n"
        ),
    }
}

fn script(lines: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.lines().count() < lines {
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
    let t = Instant::now();
    let model = model();
    println!("model: {} nodes, built in {}", model.len(), ms(t.elapsed()));
    let ctx = SqlContext::new(Dialect::Postgres, Some(Scope::new(model)));
    let text = script(5000);
    let script = parse(&text, Dialect::Postgres);
    println!(
        "script: {} lines, {} KB, {} statements",
        text.lines().count(),
        text.len() / 1024,
        script.statements().count()
    );
    let profile = InspectionProfile::default();
    let first = Analyzer::new()
        .analyze(&script, &ctx, &|| false)
        .expect("not cancelled");
    let problems = first.problems(&profile);
    println!("problems: {}", problems.len());
    assert!(
        problems.is_empty(),
        "the generated script is clean: {:?}",
        &problems[..problems.len().min(3)]
    );

    let (median, max) = measure(20, || {
        let a = Analyzer::new().analyze(&script, &ctx, &|| false);
        black_box(a);
    });
    println!(
        "analysis on open (parsed): median {}, max {}",
        ms(median),
        ms(max)
    );
    let (median, max) = measure(20, || {
        let s = parse(&text, Dialect::Postgres);
        let a = Analyzer::new().analyze(&s, &ctx, &|| false);
        black_box(a);
    });
    println!(
        "parse and analysis on open: median {}, max {}",
        ms(median),
        ms(max)
    );

    // A keystroke in the middle statement, then its removal, alternately.
    let mut analyzer = Analyzer::new();
    analyzer.analyze(&script, &ctx, &|| false);
    let at = text.len() / 2;
    let at = text[at..]
        .find("where")
        .map_or(at, |i| at + i + "where ".len());
    let typed = script.reparse(&TextEdit {
        range: at..at,
        text: "x".into(),
    });
    let mut current = script.clone();
    let mut toggle = false;
    let before = analyzer.analysed();
    let runs = 200;
    let (median, max) = measure(runs, || {
        toggle = !toggle;
        let edit = if toggle {
            TextEdit {
                range: at..at,
                text: "x".into(),
            }
        } else {
            TextEdit {
                range: at..at + 1,
                text: String::new(),
            }
        };
        current = current.reparse(&edit);
        let a = analyzer.analyze(&current, &ctx, &|| false);
        black_box((a.map(|a| a.problems(&profile).len()),));
    });
    println!(
        "keystroke: re-parse, analysis, problems: median {}, max {} ({} statements analysed per keystroke)",
        ms(median),
        ms(max),
        (analyzer.analysed() - before) as f64 / runs as f64
    );
    black_box(typed);
}
