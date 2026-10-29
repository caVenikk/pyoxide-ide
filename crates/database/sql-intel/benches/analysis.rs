//! Diagnostics benchmarks of ADR 0022 (`docs/performance.md`): a generated
//! 5000-line script bound to a model of 10 000 tables, analysed on open
//! (resolution and inspections of every statement) and after a keystroke
//! (re-parse and analysis of the edited statement, the others from the
//! cache). Run with `cargo bench -p sql-intel`; results go to
//! `replica/perf.md`.
use std::hint::black_box;
use std::time::Instant;

use sql_intel::{Analyzer, InspectionProfile, Scope, SqlContext};
use sql_syntax::{Dialect, TextEdit, parse};

mod common;
use common::{measure, model, ms, script};

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
