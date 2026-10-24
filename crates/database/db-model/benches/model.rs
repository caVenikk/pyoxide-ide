//! Metadata model benchmarks of ADR 0021 and 0022 on a synthetic schema of
//! 10 000 tables: building the model, storing and loading the disk cache
//! (target: load under 150 ms off the UI thread), and 10 000 name lookups
//! (target: under 5 ms). Run with `cargo bench -p db-model`; results go to
//! `replica/perf.md`.
use std::hint::black_box;
use std::time::{Duration, Instant};

use db_core::DataSourceId;
use db_model::{Model, ModelCache, NameMatch, NodeKind};

#[path = "../tests/it/fixture.rs"]
mod fixture;

/// The best, the median and the worst of `runs` timings of `f`; what `f`
/// returns is dropped after the clock stops. On a busy machine the best is
/// the closest to the cost of the work itself.
fn measure<T>(runs: usize, mut f: impl FnMut() -> T) -> [Duration; 3] {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            let value = black_box(f());
            let elapsed = t.elapsed();
            drop(value);
            elapsed
        })
        .collect();
    times.sort();
    [times[0], times[runs / 2], times[runs - 1]]
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn report(what: &str, [best, median, worst]: [Duration; 3]) {
    println!(
        "{what:<40} best {:>11}  median {:>11}  worst {:>11}",
        ms(best),
        ms(median),
        ms(worst)
    );
}

fn main() {
    const TABLES: usize = 10_000;
    let ds = DataSourceId::new("bench");
    let snapshot = fixture::synthetic(TABLES);
    let model = Model::from_snapshot(ds.clone(), snapshot.clone());
    println!(
        "synthetic model: {TABLES} tables, {} schemas, {} nodes",
        model.schemas().len(),
        model.len()
    );

    let mut copies: Vec<_> = (0..10).map(|_| snapshot.clone()).collect();
    report(
        "build from snapshot",
        measure(10, || {
            Model::from_snapshot(ds.clone(), copies.pop().unwrap())
        }),
    );

    let dir = std::env::temp_dir().join(format!("db-model-bench-{}", std::process::id()));
    let cache = ModelCache::new(&dir);
    report(
        "cache store (serialize, write, rename)",
        measure(10, || cache.store(&model).unwrap()),
    );
    let size = std::fs::metadata(cache.model_path(&ds)).unwrap().len();
    println!("cache file: {:.1} MB", size as f64 / 1e6);
    report(
        "cache load (read, parse, build)",
        measure(10, || cache.load(&ds).unwrap()),
    );
    assert_eq!(cache.load(&ds).unwrap(), model);
    #[derive(serde::Deserialize)]
    struct Schemas {
        schemas: Vec<db_core::schema::SchemaSnapshot>,
    }
    let bytes = std::fs::read(cache.model_path(&ds)).unwrap();
    report(
        "  of which JSON parse",
        measure(10, || {
            serde_json::from_slice::<Schemas>(&bytes).unwrap().schemas
        }),
    );

    let names: Vec<String> = (0..TABLES).map(|t| format!("table_{t:05}")).collect();
    let upper: Vec<String> = names.iter().map(|n| n.to_uppercase()).collect();
    report(
        "10 000 lookups, exact",
        measure(50, || {
            for name in &names {
                black_box(model.find("app", name, NameMatch::Exact, &NodeKind::RELATIONS));
            }
        }),
    );
    report(
        "10 000 lookups, ignoring case",
        measure(50, || {
            for name in &upper {
                black_box(model.find("APP", name, NameMatch::IgnoreCase, &NodeKind::RELATIONS));
            }
        }),
    );
    report(
        "10 000 lookups, table then column",
        measure(50, || {
            for name in &names {
                let table = model
                    .find("app", name, NameMatch::Exact, &NodeKind::RELATIONS)
                    .unwrap();
                black_box(table.child("updated_at", NameMatch::Exact, &[NodeKind::Column]));
            }
        }),
    );
    let matcher = name_match::Matcher::new("tbl99", name_match::MatchingMode::IgnoreCase);
    report(
        "name search over all objects",
        measure(5, || model.search(&matcher, &[], 100).len()),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
