//! Light-weight reading of the start of a statement. Used only to decide
//! transaction bookkeeping and how to stop a cursor, never to rewrite SQL.

/// What a statement does to the explicit transaction of a manual session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxEffect {
    None,
    Begin,
    End,
}

/// The first `max` words of a statement, lower case. Whitespace, `--` line
/// comments, nested `/* */` comments and opening parentheses are skipped.
pub(crate) fn leading_words(sql: &str, max: usize) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut words = Vec::new();
    let mut i = 0;
    while i < bytes.len() && words.len() < max {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == b'(' || b == b';' {
            i += 1;
        } else if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(bytes, i);
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            words.push(sql[start..i].to_ascii_lowercase());
        } else {
            break;
        }
    }
    words
}

fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    i
}

const DML: [&str; 4] = ["insert", "update", "delete", "merge"];

/// Whether the statement changes data and may return rows (`INSERT ...
/// RETURNING`, or a `WITH` query with a data-modifying part). A cancel that
/// arrives while the server commits such a statement rolls the change back,
/// so its cursor is read to the end instead of being cancelled.
pub(crate) fn modifies_data(sql: &str) -> bool {
    let first = leading_words(sql, 1);
    match first.first().map(String::as_str) {
        Some(w) if DML.contains(&w) => true,
        Some("with") => with_modifies(sql),
        _ => false,
    }
}

/// A `WITH` query that changes data: a common table expression that starts
/// with `INSERT|UPDATE|DELETE|MERGE`, or such a main statement after the list.
/// Literals, quoted identifiers and comments are skipped, and the `UPDATE` of
/// `FOR [NO KEY] UPDATE` does not count.
fn with_modifies(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    let mut previous = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            q @ (b'\'' | b'"') => i = skip_quoted(bytes, i, q),
            b'$' => i = skip_dollar_quote(bytes, i),
            b'(' => {
                depth += 1;
                let rest = leading_words(&sql[i + 1..], 1);
                if rest.first().is_some_and(|w| DML.contains(&w.as_str())) {
                    return true;
                }
                previous.clear();
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                previous.clear();
                i += 1;
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = sql[start..i].to_ascii_lowercase();
                let locking = word == "update" && (previous == "for" || previous == "key");
                if depth == 0 && DML.contains(&word.as_str()) && !locking {
                    return true;
                }
                previous = word;
            }
            _ => i += 1,
        }
    }
    false
}

/// Can the statement be the query of `DECLARE CURSOR`? Only row-returning
/// queries without data changes can.
pub(crate) fn is_declarable(sql: &str) -> bool {
    let first = leading_words(sql, 1);
    matches!(
        first.first().map(String::as_str),
        Some("select" | "values" | "table" | "with")
    ) && !modifies_data(sql)
}

/// Skip a `'...'` or `"..."` token; a doubled quote is an escaped quote.
fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Skip a `$tag$ ... $tag$` string. A `$1` parameter is not a dollar quote.
fn skip_dollar_quote(bytes: &[u8], start: usize) -> usize {
    let mut j = start + 1;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    let tag_ok =
        bytes.get(j) == Some(&b'$') && bytes.get(start + 1).is_none_or(|c| !c.is_ascii_digit());
    if !tag_ok {
        return start + 1;
    }
    let tag = &bytes[start..=j];
    let mut i = j + 1;
    while i + tag.len() <= bytes.len() {
        if &bytes[i..i + tag.len()] == tag {
            return i + tag.len();
        }
        i += 1;
    }
    bytes.len()
}

/// Does running this statement start or end an explicit transaction?
pub(crate) fn tx_effect(sql: &str) -> TxEffect {
    let words = leading_words(sql, 6);
    let tail: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
    match words.first().map(String::as_str) {
        Some("begin") => TxEffect::Begin,
        Some("start") if tail.first() == Some(&"transaction") => TxEffect::Begin,
        Some("commit" | "end" | "abort") => ends(&tail),
        Some("rollback") => {
            if tail.contains(&"to") {
                TxEffect::None
            } else {
                ends(&tail)
            }
        }
        Some("prepare") if tail.first() == Some(&"transaction") => TxEffect::End,
        _ => TxEffect::None,
    }
}

fn ends(tail: &[&str]) -> TxEffect {
    // COMMIT PREPARED runs outside a transaction; AND CHAIN starts the next one.
    let chained = tail.contains(&"chain") && !tail.contains(&"no");
    if tail.contains(&"prepared") || chained {
        TxEffect::None
    } else {
        TxEffect::End
    }
}

/// Statements that control the transaction themselves need no implicit BEGIN.
pub(crate) fn needs_implicit_begin(sql: &str) -> bool {
    let first = leading_words(sql, 1);
    !matches!(
        first.first().map(String::as_str),
        Some("begin" | "start" | "commit" | "end" | "abort" | "rollback")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_words_skip_noise() {
        assert_eq!(leading_words("  SELECT 1", 2), ["select"]);
        assert_eq!(leading_words("-- hi\nSelect a", 1), ["select"]);
        assert_eq!(
            leading_words("/* a /* nested */ b */ delete from t", 1),
            ["delete"]
        );
        assert_eq!(leading_words("(select 1) union (select 2)", 3), ["select"]);
        assert_eq!(
            leading_words("commit and chain", 5),
            ["commit", "and", "chain"]
        );
        assert!(leading_words("", 3).is_empty());
        assert!(leading_words("/* unterminated", 3).is_empty());
    }

    #[test]
    fn data_modifying_statements() {
        assert!(modifies_data("INSERT INTO t VALUES (1) RETURNING id"));
        assert!(modifies_data("  update t set a = 1 returning a"));
        assert!(modifies_data("delete from t returning *"));
        assert!(modifies_data(
            "merge into t using s on true when matched then delete"
        ));
        assert!(modifies_data(
            "WITH ins AS (INSERT INTO t VALUES (1) RETURNING id) SELECT * FROM ins"
        ));
        assert!(modifies_data(
            "with a as (select 1), d as materialized ( /* c */ delete from t returning *) select * from d"
        ));
        // the main statement after the CTE list
        assert!(modifies_data(
            "with x as (select 1 as n) insert into t select n from x returning *"
        ));
        assert!(modifies_data(
            "with x as (select 1 as n) update t set a = n from x returning *"
        ));
        assert!(!modifies_data("select * from t"));
        assert!(!modifies_data("with x as (select 1) select * from x"));
        // row locks are not data changes
        assert!(!modifies_data(
            "with x as (select 1) select * from t for update"
        ));
        assert!(!modifies_data(
            "with x as (select 1) select * from t for no key update skip locked"
        ));
        // words inside literals, identifiers, comments and parameters do not count
        assert!(!modifies_data(
            "with x as (select * from audit where action = 'delete') select * from x"
        ));
        assert!(!modifies_data(
            "with x as (select 1 as \"(delete\") select * from x where a = $1"
        ));
        assert!(!modifies_data(
            "with x as (select $$(insert$$) select * from x -- (update"
        ));
        assert!(!modifies_data(
            "with x as (select 'it''s (delete') select * from x"
        ));
        assert!(!modifies_data("-- insert\nselect 1"));
        assert!(!modifies_data("show all"));
    }

    #[test]
    fn declarable_statements() {
        assert!(is_declarable("select 1"));
        assert!(is_declarable("  SELECT * FROM t WHERE a = $1 FOR UPDATE"));
        assert!(is_declarable("(select 1) union (select 2)"));
        assert!(is_declarable("values (1), (2)"));
        assert!(is_declarable("table t"));
        assert!(is_declarable("with x as (select 1) select * from x"));
        assert!(is_declarable("-- c\nselect 1"));
        assert!(!is_declarable("insert into t values (1) returning id"));
        assert!(!is_declarable(
            "with x as (delete from t returning *) select * from x"
        ));
        assert!(!is_declarable("explain select 1"));
        assert!(!is_declarable("show all"));
        assert!(!is_declarable("call p()"));
        assert!(!is_declarable("update t set a = 1 returning a"));
        assert!(!is_declarable(""));
    }

    #[test]
    fn transaction_effects() {
        assert_eq!(tx_effect("BEGIN"), TxEffect::Begin);
        assert_eq!(
            tx_effect("begin isolation level serializable"),
            TxEffect::Begin
        );
        assert_eq!(tx_effect("START TRANSACTION"), TxEffect::Begin);
        assert_eq!(tx_effect("start"), TxEffect::None);
        assert_eq!(tx_effect("COMMIT"), TxEffect::End);
        assert_eq!(tx_effect("end"), TxEffect::End);
        assert_eq!(tx_effect("abort"), TxEffect::End);
        assert_eq!(tx_effect("rollback"), TxEffect::End);
        assert_eq!(tx_effect("rollback work"), TxEffect::End);
        assert_eq!(tx_effect("rollback to savepoint a"), TxEffect::None);
        assert_eq!(tx_effect("ROLLBACK TO a"), TxEffect::None);
        assert_eq!(tx_effect("commit prepared 'x'"), TxEffect::None);
        assert_eq!(tx_effect("commit and chain"), TxEffect::None);
        assert_eq!(tx_effect("commit and no chain"), TxEffect::End);
        assert_eq!(tx_effect("prepare transaction 'x'"), TxEffect::End);
        assert_eq!(tx_effect("prepare foo as select 1"), TxEffect::None);
        assert_eq!(tx_effect("select 1"), TxEffect::None);
        assert_eq!(tx_effect("-- note\ncommit"), TxEffect::End);
    }

    #[test]
    fn implicit_begin_is_skipped_for_control_statements() {
        assert!(needs_implicit_begin("select 1"));
        assert!(needs_implicit_begin("insert into t values (1)"));
        assert!(!needs_implicit_begin("commit"));
        assert!(!needs_implicit_begin("ROLLBACK"));
        assert!(!needs_implicit_begin("begin"));
        assert!(!needs_implicit_begin("-- c\nstart transaction"));
    }
}
