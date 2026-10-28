//! Completion benchmark of ADR 0022 (`docs/performance.md`): the time from
//! a keystroke (the text changes) to the items of a request, on a generated
//! 5000-line script bound to a model of 10 000 tables. Each scenario types
//! a statement into the middle of the script, asks for completion at the
//! caret and removes the statement again. Run with
//! `cargo bench -p sql-intel --bench completion`; results go to
//! `replica/perf.md`.
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use code_intel::{CodeIntel, CompletionTrigger, DocumentId, Rope, positions};
use sql_intel::{FixedScope, Scope, SqlCodeIntel, SqlContext};
use sql_syntax::Dialect;

mod common;
use common::{model, ms, script};

/// Where the statement goes: the first statement boundary at or after the
/// middle of the script.
fn middle(text: &str) -> usize {
    let half = text.len() / 2;
    text[half..].find(";\n").map_or(half, |i| half + i + 2)
}

/// The `q`-quantile (0..1) of the sorted timings.
fn quantile(times: &[Duration], q: f64) -> Duration {
    let i = ((times.len() as f64 * q).ceil() as usize).clamp(1, times.len()) - 1;
    times[i]
}

fn main() {
    let t = Instant::now();
    let model = model();
    println!("model: {} nodes, built in {}", model.len(), ms(t.elapsed()));
    let context = SqlContext::new(Dialect::Postgres, Some(Scope::new(model)));
    let base = script(5000);
    println!(
        "script: {} lines, {} KB",
        base.lines().count(),
        base.len() / 1024
    );
    let intel = SqlCodeIntel::new(Arc::new(FixedScope(context)));
    let doc = DocumentId::new("/bench/script.sql");
    let mut version = 1;
    intel.did_open(&doc, "sql", version, &Rope::from_str(&base));

    // The first request parses the script and indexes what it creates.
    let at = middle(&base);
    let (text, caret) = with_statement(&base, at, "select c.| from customers c;\n");
    version += 1;
    let t = Instant::now();
    intel.did_change(&doc, version, &[], &Rope::from_str(&text));
    let items =
        futures::executor::block_on(intel.completion(&doc, caret, CompletionTrigger::Invoked))
            .expect("an answer");
    println!(
        "first request after open: {} ({} items)",
        ms(t.elapsed()),
        items.items.len()
    );

    let scenarios: [(&str, &str); 10] = [
        ("columns of an alias", "select c.| from customers c;\n"),
        (
            "columns of an alias, prefix",
            "select c.na| from customers c;\n",
        ),
        ("tables, no prefix", "select * from |;\n"),
        ("tables, prefix", "select * from cust|;\n"),
        ("tables, prefix matching all", "select * from t|;\n"),
        (
            "columns before FROM",
            "select | from customers c join orders o on o.customer_id = c.id;\n",
        ),
        (
            "join condition",
            "select * from customers c join orders o on |;\n",
        ),
        (
            "CTE columns",
            "with x as (select id, name from customers) select | from x;\n",
        ),
        ("functions and keywords", "select coun| from customers;\n"),
        ("keywords at the statement start", "|\n"),
    ];
    let runs = 200;
    let mut all: Vec<Duration> = Vec::new();
    for (name, statement) in scenarios {
        let mut times: Vec<Duration> = Vec::with_capacity(runs);
        let mut count = 0;
        for _ in 0..runs {
            let (text, caret) = with_statement(&base, at, statement);
            let rope = Rope::from_str(&text);
            version += 1;
            let t = Instant::now();
            intel.did_change(&doc, version, &[], &rope);
            let answer = futures::executor::block_on(intel.completion(
                &doc,
                caret,
                CompletionTrigger::Invoked,
            ))
            .expect("an answer");
            times.push(t.elapsed());
            count = answer.items.len();
            black_box(answer);
            // The statement is removed again.
            version += 1;
            intel.did_change(&doc, version, &[], &Rope::from_str(&base));
        }
        times.sort();
        println!(
            "{name}: median {}, p99 {}, max {} ({count} items)",
            ms(times[runs / 2]),
            ms(quantile(&times, 0.99)),
            ms(times[runs - 1]),
        );
        all.extend(times);
    }
    all.sort();
    println!(
        "all scenarios: median {}, p99 {}, max {} ({} requests)",
        ms(all[all.len() / 2]),
        ms(quantile(&all, 0.99)),
        ms(all[all.len() - 1]),
        all.len()
    );
}

/// `base` with `statement` (its `|` is the caret) inserted at `at`, and the
/// caret as a point.
fn with_statement(base: &str, at: usize, statement: &str) -> (String, code_intel::Point) {
    let bar = statement.find('|').expect("a caret");
    let mut text = String::with_capacity(base.len() + statement.len());
    text.push_str(&base[..at]);
    text.push_str(&statement.replace('|', ""));
    text.push_str(&base[at..]);
    let rope = Rope::from_str(&text);
    let caret = positions::offset_to_point(&rope, rope.byte_to_char(at + bar));
    (text, caret)
}
