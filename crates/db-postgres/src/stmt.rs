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

/// Whether the statement changes data and may return rows (`INSERT ...
/// RETURNING`). Stopping such a statement half-way would roll the change
/// back, so its cursor is read to the end instead of being cancelled.
pub(crate) fn modifies_data(sql: &str) -> bool {
    const DML: [&str; 4] = ["insert", "update", "delete", "merge"];
    let first = leading_words(sql, 1);
    match first.first().map(String::as_str) {
        Some(w) if DML.contains(&w) => true,
        // A CTE may hold the data-modifying part: look at every word.
        Some("with") => all_words(sql).iter().any(|w| DML.contains(&w.as_str())),
        _ => false,
    }
}

fn all_words(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut words = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
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
            i += 1;
        }
    }
    words
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
        assert!(!modifies_data("select * from t"));
        assert!(!modifies_data("with x as (select 1) select * from x"));
        assert!(!modifies_data("-- insert\nselect 1"));
        assert!(!modifies_data("show all"));
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
