//! Corpora of valid statements that are not queries: data modification, DDL
//! and procedural code. Each block parses without an error, as one unit whose
//! statement node has the kind named by `first`, and prints back to its text;
//! `sqlparser` (a test oracle, ADR 0022) accepts the blocks it is listed for.
use sql_syntax::{Dialect, Script, SyntaxKind, parse, split};
use sqlparser::dialect::{
    Dialect as OracleDialect, GenericDialect, PostgreSqlDialect, SQLiteDialect,
};
use sqlparser::parser::Parser;

const DML: &str = include_str!("corpus/dml.sql");
const DDL: &str = include_str!("corpus/ddl.sql");
const PLSQL: &str = include_str!("corpus/plsql.sql");
const PLPGSQL: &str = include_str!("corpus/plpgsql.sql");

pub struct Case {
    pub text: String,
    pub dialects: Vec<Dialect>,
    pub first: Option<String>,
    pub sqlparser: Vec<Dialect>,
}

fn dialect_list(list: &str) -> Vec<Dialect> {
    list.split(',')
        .map(|d| {
            Dialect::from_id(d.trim()).unwrap_or_else(|| panic!("unknown dialect {d:?} in corpus"))
        })
        .collect()
}

/// The blocks of a corpus file (separated by blank lines; a block without
/// code, the header, is skipped) with their directives.
pub fn cases(src: &str, default: &[Dialect]) -> Vec<Case> {
    let mut out = Vec::new();
    for block in src.split("\n\n") {
        let block = block.trim();
        let code = block
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with("--"));
        if !code {
            continue;
        }
        let mut dialects = default.to_vec();
        let mut first = None;
        let mut sqlparser = Vec::new();
        let mut text = Vec::new();
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("-- dialects:") {
                dialects = dialect_list(rest);
            } else if let Some(rest) = line.strip_prefix("-- first:") {
                first = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("-- sqlparser:") {
                sqlparser = dialect_list(rest);
            } else {
                text.push(line);
            }
        }
        out.push(Case {
            text: text.join("\n"),
            dialects,
            first,
            sqlparser,
        });
    }
    out
}

/// The kind of the first statement node of a one-unit script.
pub fn first_kind(script: &Script) -> Option<SyntaxKind> {
    script
        .statements()
        .next()
        .and_then(|r| r.child_nodes().next())
        .map(|n| n.kind())
}

/// The problems of `case` in `dialect`: errors, unit count, the kind of the
/// statement, the print.
pub fn problems(case: &Case, dialect: Dialect) -> Vec<String> {
    let text = &case.text;
    let script = parse(text, dialect);
    let mut out: Vec<String> = script
        .errors()
        .iter()
        .map(|e| format!("{:?}: {}", e.range, e.message(text)))
        .collect();
    let units = script.statements().count();
    if units != 1 {
        out.push(format!("{units} units"));
    }
    let kind = first_kind(&script);
    match (&case.first, kind) {
        (Some(want), Some(got)) if format!("{got:?}") != *want => {
            out.push(format!("first node is {got:?}, want {want}"));
        }
        (None, Some(SyntaxKind::Opaque)) => out.push("opaque".to_string()),
        (_, None) => out.push("no statement node".to_string()),
        _ => {}
    }
    if script.print() != *text {
        out.push("print differs".to_string());
    }
    let statements = split(text, dialect);
    if statements.len() != 1 {
        out.push(format!("{} split statements", statements.len()));
    }
    out
}

pub fn check_corpus(src: &str, default: &[Dialect], min: usize) -> usize {
    let all = cases(src, default);
    let mut failures = Vec::new();
    for c in &all {
        for &d in &c.dialects {
            let p = problems(c, d);
            if !p.is_empty() {
                failures.push(format!("{d:?}: {}\n  {}", c.text, p.join("\n  ")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(all.len() >= min, "{} blocks", all.len());
    all.len()
}

pub fn check_sqlparser(src: &str) {
    let mut failures = Vec::new();
    for c in cases(src, &[]) {
        for &d in &c.sqlparser {
            let oracle: &dyn OracleDialect = match d {
                Dialect::Postgres => &PostgreSqlDialect {},
                Dialect::Sqlite => &SQLiteDialect {},
                _ => &GenericDialect {},
            };
            if let Err(e) = Parser::parse_sql(oracle, &c.text) {
                failures.push(format!("{d:?}: {}\n  {e}", c.text));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn data_modification_parses_without_errors() {
    check_corpus(DML, &Dialect::ALL, 80);
}

#[test]
fn sqlparser_accepts_the_data_modification_it_is_listed_for() {
    check_sqlparser(DML);
}

#[test]
fn ddl_parses_without_errors() {
    check_corpus(DDL, &Dialect::ALL, 150);
}

#[test]
fn sqlparser_accepts_the_ddl_it_is_listed_for() {
    check_sqlparser(DDL);
}

#[test]
fn plsql_blocks_and_routines_parse_without_errors() {
    check_corpus(PLSQL, &[Dialect::Oracle], 40);
}

#[test]
fn plpgsql_routines_parse_without_errors() {
    check_corpus(PLPGSQL, &[Dialect::Postgres, Dialect::Generic], 40);
}
