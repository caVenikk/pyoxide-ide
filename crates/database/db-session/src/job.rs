use std::ops::Range;

use db_core::Value;
use sql_syntax::{StatementKind, TokenKind};

/// What a session runs: statements of a script, one after another.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub(crate) source: JobSource,
    pub(crate) on_error: ErrorPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JobSource {
    /// Split by the session with the dialect of its connection.
    Script(String),
    Statements(Vec<JobStatement>),
}

/// One statement of a job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobStatement {
    /// The text sent to the server.
    pub text: String,
    /// Byte range in the script the statement came from; error positions are
    /// reported in the same coordinates.
    pub range: Range<usize>,
    pub kind: StatementKind,
    /// Changes data ([`sql_syntax::Classification::writes`]).
    pub writes: bool,
    /// Bound positionally (Oracle `:1`, PostgreSQL `$1`, SQLite `?1`).
    pub params: Vec<Value>,
}

impl From<sql_syntax::Statement> for JobStatement {
    fn from(s: sql_syntax::Statement) -> Self {
        Self {
            text: s.text,
            range: s.range,
            kind: s.kind,
            writes: s.writes,
            params: Vec::new(),
        }
    }
}

/// What a job does after a statement failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorPolicy {
    /// Stop: the remaining statements do not run.
    #[default]
    Stop,
    /// Run the remaining statements.
    Continue,
}

impl Job {
    /// A script, split into statements by the session with its connection's
    /// dialect (`sql_syntax::split`).
    pub fn script(text: impl Into<String>) -> Self {
        Self {
            source: JobSource::Script(text.into()),
            on_error: ErrorPolicy::Stop,
        }
    }

    /// Statements the caller split already (the statement at the caret, a
    /// selection).
    pub fn statements(statements: impl IntoIterator<Item = impl Into<JobStatement>>) -> Self {
        Self {
            source: JobSource::Statements(statements.into_iter().map(Into::into).collect()),
            on_error: ErrorPolicy::Stop,
        }
    }

    pub fn on_error(mut self, policy: ErrorPolicy) -> Self {
        self.on_error = policy;
        self
    }

    pub(crate) fn resolve(self, dialect: db_core::Dialect) -> Vec<JobStatement> {
        match self.source {
            JobSource::Statements(s) => s,
            JobSource::Script(text) => sql_syntax::split(&text, syntax_dialect(dialect))
                .into_iter()
                .map(JobStatement::from)
                .collect(),
        }
    }
}

/// The text layer's dialect of an engine dialect.
pub fn syntax_dialect(dialect: db_core::Dialect) -> sql_syntax::Dialect {
    match dialect {
        db_core::Dialect::Oracle => sql_syntax::Dialect::Oracle,
        db_core::Dialect::Postgres => sql_syntax::Dialect::Postgres,
        db_core::Dialect::Sqlite => sql_syntax::Dialect::Sqlite,
    }
}

/// What a transaction control statement does to the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxEffect {
    /// `BEGIN`, `START TRANSACTION`, `SET TRANSACTION`.
    Begins,
    /// `COMMIT`, `ROLLBACK` (not `ROLLBACK TO`), `END`, `ABORT`.
    Ends,
    /// `SAVEPOINT`, `RELEASE`, `ROLLBACK TO`, anything else.
    Keeps,
}

/// The first words of a statement, lower case, comments skipped.
pub(crate) fn leading_words(text: &str, dialect: db_core::Dialect, n: usize) -> Vec<String> {
    sql_syntax::lex(text, syntax_dialect(dialect))
        .into_iter()
        .filter(|t| !t.is_trivia())
        .take(n)
        .take_while(|t| t.kind == TokenKind::Word)
        .map(|t| t.text(text).to_ascii_lowercase())
        .collect()
}

pub(crate) fn tx_effect(statement: &JobStatement, dialect: db_core::Dialect) -> TxEffect {
    if statement.kind != StatementKind::Tcl {
        return TxEffect::Keeps;
    }
    let words = leading_words(&statement.text, dialect, 4);
    let has = |w: &str| words.iter().any(|x| x == w);
    match words.first().map(String::as_str) {
        Some("begin" | "start" | "set") => TxEffect::Begins,
        Some("rollback") if has("to") => TxEffect::Keeps,
        Some("commit" | "rollback" | "end" | "abort") if !has("prepared") && !has("chain") => {
            TxEffect::Ends
        }
        _ => TxEffect::Keeps,
    }
}

/// Statements after which the current schema may have changed (`SET
/// search_path`, `ALTER SESSION SET CURRENT_SCHEMA`, `USE`).
pub(crate) fn may_change_schema(statement: &JobStatement, dialect: db_core::Dialect) -> bool {
    if statement.kind != StatementKind::Other && statement.kind != StatementKind::Ddl {
        return false;
    }
    let words = leading_words(&statement.text, dialect, 4);
    match words.first().map(String::as_str) {
        Some("set") => words.iter().any(|w| w == "search_path" || w == "schema"),
        Some("alter") => words.get(1).is_some_and(|w| w == "session"),
        Some("use") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_core::Dialect;

    fn stmt(text: &str, dialect: Dialect) -> JobStatement {
        let mut v = Job::script(text).resolve(dialect);
        assert_eq!(v.len(), 1, "{text}");
        v.remove(0)
    }

    #[test]
    fn scripts_split_by_the_connection_dialect() {
        let pg = Job::script("select 1; select $$a;b$$; insert into t values (1)")
            .resolve(Dialect::Postgres);
        assert_eq!(pg.len(), 3);
        assert_eq!(pg[1].text, "select $$a;b$$");
        assert!(pg[2].writes);
        let ora = Job::script("begin null; end;\n/\nselect 1 from dual").resolve(Dialect::Oracle);
        assert_eq!(ora.len(), 2);
        assert_eq!(ora[0].kind, StatementKind::AnonymousBlock);
    }

    #[test]
    fn transaction_effects() {
        let pg = Dialect::Postgres;
        assert_eq!(tx_effect(&stmt("BEGIN", pg), pg), TxEffect::Begins);
        assert_eq!(tx_effect(&stmt("commit", pg), pg), TxEffect::Ends);
        assert_eq!(tx_effect(&stmt("rollback", pg), pg), TxEffect::Ends);
        assert_eq!(
            tx_effect(&stmt("rollback to savepoint a", pg), pg),
            TxEffect::Keeps
        );
        assert_eq!(tx_effect(&stmt("savepoint a", pg), pg), TxEffect::Keeps);
        assert_eq!(
            tx_effect(&stmt("commit and chain", pg), pg),
            TxEffect::Keeps
        );
        assert_eq!(tx_effect(&stmt("select 1", pg), pg), TxEffect::Keeps);
    }

    #[test]
    fn schema_changing_statements() {
        let pg = Dialect::Postgres;
        assert!(may_change_schema(&stmt("set search_path to x", pg), pg));
        assert!(!may_change_schema(&stmt("set work_mem = 1", pg), pg));
        assert!(!may_change_schema(&stmt("select 1", pg), pg));
        let ora = Dialect::Oracle;
        assert!(may_change_schema(
            &stmt("alter session set current_schema = hr", ora),
            ora
        ));
    }
}
