//! Parameters of a console statement: what the statement asks for (named
//! `:id`, positional `?`, `$1`, `:1`), the values the person typed, and the
//! statement rewritten to the engine's positional form with the values
//! bound. A value never goes into the SQL text.

use std::collections::HashMap;

use db_core::Value;
use db_session::JobStatement;
use sql_syntax::{Dialect, ParamKind, Statement, StatementKind};

/// One value the statements ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    /// What the value is remembered by: the name of a named parameter,
    /// `$n` of a numbered one, `n:?k` of the k-th placeholder of the n-th
    /// statement.
    pub key: String,
    /// What the dialog names it: `id`, `$1`, `?`.
    pub label: String,
    /// The words before it in the statement, ending in `_`: `id = _`.
    pub context: String,
}

/// Statements whose text is not searched: DDL and blocks may use `:name`
/// for something else (trigger rows, PL/SQL variables).
fn takes_parameters(kind: StatementKind) -> bool {
    !matches!(kind, StatementKind::Ddl | StatementKind::AnonymousBlock)
}

/// The values the statements ask for, in order, each once.
pub(crate) fn slots(statements: &[Statement], dialect: Dialect) -> Vec<Slot> {
    let forms = dialect.default_params();
    let mut out: Vec<Slot> = Vec::new();
    for (index, st) in statements.iter().enumerate() {
        if !takes_parameters(st.kind) {
            continue;
        }
        let params = sql_syntax::parameters(&st.text, dialect, forms);
        let tokens: Vec<_> = sql_syntax::lex_with(&st.text, dialect, forms)
            .into_iter()
            .filter(|t| !t.is_trivia())
            .collect();
        let mut positional = 0;
        for p in &params {
            let (key, label) = match (&p.kind, &p.name) {
                (ParamKind::Named, Some(name)) => (name.clone(), name.clone()),
                (ParamKind::Numbered(n), _) => (format!("${n}"), format!("${n}")),
                _ => {
                    positional += 1;
                    (format!("{index}:?{positional}"), "?".to_owned())
                }
            };
            if out.iter().any(|s| s.key == key) {
                continue;
            }
            let at = tokens.iter().position(|t| t.start == p.range.start);
            let before: Vec<&str> = at
                .map(|at| {
                    tokens[at.saturating_sub(2)..at]
                        .iter()
                        .map(|t| t.text(&st.text))
                        .collect()
                })
                .unwrap_or_default();
            let mut context = before.join(" ");
            if !context.is_empty() {
                context.push(' ');
            }
            context.push('_');
            out.push(Slot {
                key,
                label,
                context,
            });
        }
    }
    out
}

/// The placeholder of the `n`-th (from 1) value in the engine's positional
/// form.
fn placeholder(dialect: Dialect, n: usize) -> String {
    match dialect {
        Dialect::Oracle => format!(":{n}"),
        Dialect::Sqlite => format!("?{n}"),
        Dialect::Postgres | Dialect::Generic => format!("${n}"),
    }
}

/// The statement to send: its parameters replaced by the engine's
/// positional placeholders, the values (by slot key) bound in the order the
/// placeholders appear.
pub(crate) fn bind(
    st: &Statement,
    index: usize,
    dialect: Dialect,
    values: &HashMap<String, String>,
) -> JobStatement {
    let mut job = JobStatement::from(st.clone());
    if !takes_parameters(st.kind) {
        return job;
    }
    let forms = dialect.default_params();
    let params = sql_syntax::parameters(&st.text, dialect, forms);
    if params.is_empty() {
        return job;
    }
    let mut keys: Vec<String> = Vec::new();
    let mut text = String::with_capacity(st.text.len());
    let mut copied = 0;
    let mut positional = 0;
    for p in &params {
        let key = match (&p.kind, &p.name) {
            (ParamKind::Named, Some(name)) => name.clone(),
            (ParamKind::Numbered(n), _) => format!("${n}"),
            _ => {
                positional += 1;
                format!("{index}:?{positional}")
            }
        };
        let n = match keys.iter().position(|k| *k == key) {
            Some(at) => at + 1,
            None => {
                keys.push(key);
                keys.len()
            }
        };
        text.push_str(&st.text[copied..p.range.start]);
        text.push_str(&placeholder(dialect, n));
        copied = p.range.end;
    }
    text.push_str(&st.text[copied..]);
    job.text = text;
    job.params = keys
        .iter()
        .map(|k| values.get(k).map_or(Value::Null, |v| parse_value(v)))
        .collect();
    job
}

/// What the person typed as a value: `null`, a number, `true`/`false`, a
/// string in single quotes (`''` is a quote), else the text as typed.
pub(crate) fn parse_value(text: &str) -> Value {
    let t = text.trim();
    if t.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if t.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Int(i);
    }
    let digits = t.strip_prefix(['-', '+']).unwrap_or(t);
    if let Some((whole, fraction)) = digits.split_once('.')
        && !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && !fraction.is_empty()
        && fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Value::Decimal(t.trim_start_matches('+').to_owned());
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return Value::Text(t[1..t.len() - 1].replace("''", "'"));
    }
    Value::Text(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statements(text: &str, dialect: Dialect) -> Vec<Statement> {
        sql_syntax::split(text, dialect)
    }

    #[test]
    fn named_parameters_become_positional_with_the_values_bound() {
        let st = statements("select * from t where id = :id and n = :id or s = :s", Dialect::Postgres);
        let slots = slots(&st, Dialect::Postgres);
        assert_eq!(
            slots.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["id", "s"]
        );
        assert_eq!(slots[0].context, "id = _");
        let values = HashMap::from([
            ("id".to_owned(), "7".to_owned()),
            ("s".to_owned(), "'o''k'".to_owned()),
        ]);
        let job = bind(&st[0], 0, Dialect::Postgres, &values);
        assert_eq!(job.text, "select * from t where id = $1 and n = $1 or s = $2");
        assert_eq!(job.params, [Value::Int(7), Value::Text("o'k".into())]);
        // The values are not in the text.
        assert!(!job.text.contains("o'k"));
    }

    #[test]
    fn placeholders_follow_the_engine() {
        for (dialect, text, want) in [
            (Dialect::Oracle, "select 1 from dual where a = :x", ":1"),
            (Dialect::Sqlite, "select 1 where a = ?", "?1"),
            (Dialect::Postgres, "select 1 where a = $1", "$1"),
        ] {
            let st = statements(text, dialect);
            let values = HashMap::from([
                ("x".to_owned(), "1".to_owned()),
                ("0:?1".to_owned(), "1".to_owned()),
                ("$1".to_owned(), "1".to_owned()),
            ]);
            let job = bind(&st[0], 0, dialect, &values);
            assert!(job.text.ends_with(want), "{dialect:?}: {}", job.text);
            assert_eq!(job.params.len(), 1);
        }
    }

    #[test]
    fn values_are_typed_by_what_was_written() {
        assert_eq!(parse_value("42"), Value::Int(42));
        assert_eq!(parse_value("-1.50"), Value::Decimal("-1.50".into()));
        assert_eq!(parse_value("NULL"), Value::Null);
        assert_eq!(parse_value("true"), Value::Bool(true));
        assert_eq!(parse_value("'a b'"), Value::Text("a b".into()));
        assert_eq!(parse_value("plain"), Value::Text("plain".into()));
        assert_eq!(parse_value("1.2.3"), Value::Text("1.2.3".into()));
    }

    #[test]
    fn a_statement_without_parameters_and_ddl_are_left_alone() {
        let st = statements("select 1; create table t (a int default :x)", Dialect::Postgres);
        assert!(slots(&st, Dialect::Postgres).is_empty());
        let job = bind(&st[1], 1, Dialect::Postgres, &HashMap::new());
        assert_eq!(job.text, st[1].text);
        assert!(job.params.is_empty());
    }

    #[test]
    fn question_marks_are_asked_per_statement() {
        let st = statements("select ? ; select ?", Dialect::Sqlite);
        let slots = slots(&st, Dialect::Sqlite);
        assert_eq!(slots.len(), 2);
        assert_ne!(slots[0].key, slots[1].key);
    }
}
