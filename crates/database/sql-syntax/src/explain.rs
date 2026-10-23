use crate::dialect::Dialect;
use crate::lexer::{TokenKind, lex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExplainOptions {
    /// Run the statement and report actual times and rows. Only where
    /// [`supports_analyze`] says so; ignored otherwise.
    pub analyze: bool,
}

pub fn supports_analyze(dialect: Dialect) -> bool {
    dialect == Dialect::Postgres
}

/// The statements that show the plan of `stmt`, to run one after another:
/// `EXPLAIN (FORMAT JSON)` on PostgreSQL, `EXPLAIN QUERY PLAN` on SQLite,
/// `EXPLAIN PLAN FOR` and a read of `DBMS_XPLAN` on Oracle, `EXPLAIN` on
/// Generic. A closing `;` of `stmt` is dropped. Empty when `stmt` has nothing
/// in it.
pub fn explain_statements(stmt: &str, dialect: Dialect, options: ExplainOptions) -> Vec<String> {
    let sig: Vec<_> = lex(stmt, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .collect();
    let Some(first) = sig.first() else {
        return Vec::new();
    };
    let Some(last) = sig.iter().rev().find(|t| t.kind != TokenKind::Semicolon) else {
        return Vec::new();
    };
    let s = &stmt[first.start..last.end];
    match dialect {
        Dialect::Postgres if options.analyze => vec![format!("EXPLAIN (ANALYZE, FORMAT JSON) {s}")],
        Dialect::Postgres => vec![format!("EXPLAIN (FORMAT JSON) {s}")],
        Dialect::Sqlite => vec![format!("EXPLAIN QUERY PLAN {s}")],
        Dialect::Oracle => vec![
            format!("EXPLAIN PLAN FOR {s}"),
            "SELECT plan_table_output FROM TABLE(DBMS_XPLAN.DISPLAY())".to_string(),
        ],
        Dialect::Generic => vec![format!("EXPLAIN {s}")],
    }
}
