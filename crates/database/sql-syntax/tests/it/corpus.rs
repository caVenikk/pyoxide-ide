//! Corpora of hand-written queries per dialect: valid ones parse without an
//! error, invalid ones report one, and `sqlparser` (a test oracle, ADR 0022)
//! agrees on acceptance for the PostgreSQL, SQLite and common subset.
use sql_syntax::{Dialect, SyntaxKind, parse};
use sqlparser::dialect::{GenericDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

const COMMON: &str = include_str!("corpus/common.sql");
const POSTGRES: &str = include_str!("corpus/postgres.sql");
const ORACLE: &str = include_str!("corpus/oracle.sql");
const SQLITE: &str = include_str!("corpus/sqlite.sql");
const INVALID: &str = include_str!("corpus/invalid.sql");

struct Block {
    text: String,
    /// Not given to sqlparser (`-- no-oracle`).
    no_oracle: bool,
    /// Not parsed with the generic dialect (`-- not-generic`).
    not_generic: bool,
}

/// The blocks of a corpus file: separated by blank lines, a block of only
/// comments (the header) skipped.
fn blocks(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for block in src.split("\n\n") {
        let block = block.trim();
        let code = block
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with("--"));
        if !code {
            continue;
        }
        let directive = |d: &str| {
            block
                .lines()
                .take_while(|l| l.starts_with("--"))
                .any(|l| l.trim() == d)
        };
        out.push(Block {
            text: block.to_string(),
            no_oracle: directive("-- no-oracle"),
            not_generic: directive("-- not-generic"),
        });
    }
    out
}

/// Parses `text` and returns the problems: syntax errors, a statement that is
/// not a query, more than one statement, a print that differs.
fn problems(text: &str, dialect: Dialect) -> Vec<String> {
    let script = parse(text, dialect);
    let mut out: Vec<String> = script
        .errors()
        .iter()
        .map(|e| format!("{:?}: {}", e.range, e.message(text)))
        .collect();
    let roots: Vec<_> = script.statements().collect();
    if roots.len() != 1 {
        out.push(format!("{} units", roots.len()));
    }
    if roots[0].child_nodes().next().map(|n| n.kind()) != Some(SyntaxKind::Query) {
        out.push("not a query".to_string());
    }
    if script.print() != text {
        out.push("print differs".to_string());
    }
    out
}

fn check_valid(src: &str, dialects: &[Dialect]) -> usize {
    let mut failures = Vec::new();
    let all = blocks(src);
    for b in &all {
        for &d in dialects {
            if d == Dialect::Generic && b.not_generic {
                continue;
            }
            let p = problems(&b.text, d);
            if !p.is_empty() {
                failures.push(format!("{d:?}: {}\n  {}", b.text, p.join("\n  ")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    all.len()
}

fn check_oracle(src: &str, dialects: &[&dyn sqlparser::dialect::Dialect]) {
    let mut failures = Vec::new();
    for b in blocks(src).iter().filter(|b| !b.no_oracle) {
        for d in dialects {
            if let Err(e) = Parser::parse_sql(*d, &b.text) {
                failures.push(format!("{d:?}: {}\n  {e}", b.text));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn common_queries_parse_without_errors_in_every_dialect() {
    let n = check_valid(COMMON, &Dialect::ALL);
    assert!(n >= 100, "{n} queries");
}

#[test]
fn postgres_queries_parse_without_errors() {
    let n = check_valid(POSTGRES, &[Dialect::Postgres, Dialect::Generic]);
    assert!(n >= 100, "{n} queries");
}

#[test]
fn oracle_queries_parse_without_errors() {
    let n = check_valid(ORACLE, &[Dialect::Oracle]);
    assert!(n >= 80, "{n} queries");
}

#[test]
fn sqlite_queries_parse_without_errors() {
    let n = check_valid(SQLITE, &[Dialect::Sqlite, Dialect::Generic]);
    assert!(n >= 70, "{n} queries");
}

#[test]
fn sqlparser_accepts_the_common_queries() {
    check_oracle(
        COMMON,
        &[&GenericDialect {}, &PostgreSqlDialect {}, &SQLiteDialect {}],
    );
}

#[test]
fn sqlparser_accepts_the_postgres_queries() {
    check_oracle(POSTGRES, &[&PostgreSqlDialect {}]);
}

#[test]
fn sqlparser_accepts_the_sqlite_queries() {
    check_oracle(SQLITE, &[&SQLiteDialect {}]);
}

#[test]
fn invalid_queries_are_rejected_by_both_parsers() {
    let mut failures = Vec::new();
    for b in blocks(INVALID) {
        for d in Dialect::ALL {
            if parse(&b.text, d).errors().is_empty() {
                failures.push(format!("no error in {d:?}: {}", b.text));
            }
        }
        // sqlparser's generic dialect accepts a trailing comma and `AS` as a
        // name; the engines do not.
        let oracles: [&dyn sqlparser::dialect::Dialect; 2] =
            [&PostgreSqlDialect {}, &SQLiteDialect {}];
        for o in oracles {
            if Parser::parse_sql(o, &b.text).is_ok() {
                failures.push(format!("sqlparser {o:?} accepts: {}", b.text));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
