//! Names as keys: the model keeps names exactly as the database stores them
//! and indexes them ignoring case, so both an exact lookup (PostgreSQL and
//! Oracle after the caller folded the identifier) and a case-insensitive one
//! (SQLite) are one hash lookup without allocating.

use std::cmp::Ordering;

use crate::id::StableHasher;

/// How a lookup compares names. The model does not fold identifiers: the
/// SQL layer applies the dialect's rules (ADR 0022) and asks for the exact
/// stored name, or for any case where the dialect compares that way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum NameMatch {
    /// The name as stored, byte for byte.
    #[default]
    Exact,
    /// Any letter case (Unicode lower-case mapping, char by char).
    IgnoreCase,
}

impl NameMatch {
    pub fn matches(self, stored: &str, wanted: &str) -> bool {
        match self {
            NameMatch::Exact => stored == wanted,
            NameMatch::IgnoreCase => eq_ignore_case(stored, wanted),
        }
    }
}

fn lower(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().flat_map(char::to_lowercase)
}

pub(crate) fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        return a.eq_ignore_ascii_case(b);
    }
    lower(a).eq(lower(b))
}

/// The hash of a name ignoring case; equal for names that
/// [`eq_ignore_case`] calls equal.
pub(crate) fn caseless_hash(s: &str) -> u64 {
    let mut h = StableHasher::new();
    if s.is_ascii() {
        for b in s.bytes() {
            h.bytes(&[b.to_ascii_lowercase()]);
        }
    } else {
        let mut buf = [0u8; 4];
        for c in lower(s) {
            h.bytes(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    h.finish()
}

/// The order of names in the tree: ignoring case, then exactly, so the
/// order does not depend on the order the database returned them in.
pub(crate) fn cmp_names(a: &str, b: &str) -> Ordering {
    let folded = if a.is_ascii() && b.is_ascii() {
        let fold = |b: u8| b.to_ascii_lowercase();
        a.bytes().map(fold).cmp(b.bytes().map(fold))
    } else {
        lower(a).cmp(lower(b))
    };
    folded.then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caseless_keys() {
        assert!(eq_ignore_case("Orders", "ORDERS"));
        assert!(eq_ignore_case("ÄPFEL", "äpfel"));
        assert!(!eq_ignore_case("order", "orders"));
        assert_eq!(caseless_hash("ÄPFEL"), caseless_hash("äpfel"));
        assert_eq!(caseless_hash("Orders"), caseless_hash("oRDERS"));
        assert_ne!(caseless_hash("orders"), caseless_hash("order"));
        assert!(NameMatch::Exact.matches("A", "A") && !NameMatch::Exact.matches("A", "a"));
        assert!(NameMatch::IgnoreCase.matches("A", "a"));
    }

    #[test]
    fn name_order() {
        let mut names = vec!["b", "B", "a", "A", "_x", "C"];
        names.sort_by(|a, b| cmp_names(a, b));
        assert_eq!(names, ["_x", "A", "a", "B", "b", "C"]);
    }
}
