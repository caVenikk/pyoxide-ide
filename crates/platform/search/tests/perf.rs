//! Search times on the generated 10 MB file of the performance runs
//! (scripts/perf: `x = '<90 x a>'` rows, 10 MiB). Ignored by default; run
//! with
//!
//! ```text
//! cargo test --release -p search --test perf -- --ignored --nocapture
//! ```
//!
//! Each query is searched five times; the median is printed with the match
//! count. The numbers go to replica/perf.md.

use std::time::{Duration, Instant};

use search::{CancelToken, Query, SearchOptions, to_char_ranges};

/// The 10 MB file of scripts/perf/pycharm_baseline.py.
fn huge_file() -> String {
    let row = format!("x = '{}'\n", "a".repeat(90));
    row.repeat(10 * 1024 * 1024 / row.len())
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn search_the_10_mb_file() {
    let text = huge_file();
    let plain = SearchOptions::default();
    let case = SearchOptions {
        case_sensitive: true,
        ..plain
    };
    let words = SearchOptions {
        whole_words: true,
        ..plain
    };
    let regex = SearchOptions {
        regex: true,
        case_sensitive: true,
        ..plain
    };
    let cases = [
        ("plain, no match", "zzz", plain),
        ("plain, 1 per line", "x = ", plain),
        ("plain, case-sensitive, 1 per line", "x = ", case),
        ("whole words, 1 per line", "x", words),
        ("whole words, rejected everywhere", "aaa", words),
        ("regex, 1 per line", r"'a+'$", regex),
        ("regex, line starts", "^", regex),
        ("plain, 30 per line, limit 1,000,000", "aaa", plain),
    ];
    println!(
        "10 MB file: {} bytes, {} lines",
        text.len(),
        text.lines().count()
    );
    for (name, query, options) in cases {
        let matcher = Query::new(query, options).compile().unwrap();
        let mut times = Vec::new();
        let mut convert = Vec::new();
        let mut count = 0;
        let mut truncated = false;
        for _ in 0..5 {
            let t = Instant::now();
            let found = matcher
                .find_all(&text, 0..text.len(), 1_000_000, &CancelToken::new())
                .unwrap();
            times.push(t.elapsed());
            let mut ranges = found.ranges;
            let t = Instant::now();
            to_char_ranges(&text, &mut ranges);
            convert.push(t.elapsed());
            count = ranges.len();
            truncated = found.truncated;
        }
        println!(
            "{name:<40} {count:>8}{} matches  search {:>7.2} ms  to chars {:>6.2} ms",
            if truncated { "+" } else { " " },
            median(times).as_secs_f64() * 1e3,
            median(convert).as_secs_f64() * 1e3,
        );
    }
}

#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn cancelling_stops_a_search_early() {
    let text = huge_file();
    let matcher = Query::new("a", SearchOptions::default()).compile().unwrap();
    let token = CancelToken::new();
    let cancel = token.clone();
    let t = Instant::now();
    let worker =
        std::thread::spawn(move || matcher.find_all(&text, 0..text.len(), usize::MAX, &token));
    std::thread::sleep(Duration::from_millis(5));
    cancel.cancel();
    let result = worker.join().unwrap();
    println!(
        "cancelled after 5 ms: returned after {:.2} ms, {:?}",
        t.elapsed().as_secs_f64() * 1e3,
        result.err()
    );
}
