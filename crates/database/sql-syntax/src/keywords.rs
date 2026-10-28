mod oracle;
mod postgres;
mod sqlite;

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::dialect::KeywordClass;

pub(crate) use oracle::{NON_RESERVED as ORACLE_NON_RESERVED, RESERVED as ORACLE_RESERVED};
pub(crate) use postgres::{NON_RESERVED as POSTGRES_NON_RESERVED, RESERVED as POSTGRES_RESERVED};
pub(crate) use sqlite::{NON_RESERVED as SQLITE_NON_RESERVED, RESERVED as SQLITE_RESERVED};

const LONGEST: usize = 32;

/// Looks `word` up in the union of the dialects' tables: reserved in any
/// of them is reserved. One hash lookup instead of six binary searches, for
/// highlighting.
pub(crate) fn lookup_union(word: &str) -> Option<KeywordClass> {
    static UNION: LazyLock<HashMap<&'static str, KeywordClass>> = LazyLock::new(|| {
        let mut map = HashMap::new();
        for table in [
            POSTGRES_NON_RESERVED,
            ORACLE_NON_RESERVED,
            SQLITE_NON_RESERVED,
        ] {
            for w in table {
                map.insert(*w, KeywordClass::NonReserved);
            }
        }
        for table in [POSTGRES_RESERVED, ORACLE_RESERVED, SQLITE_RESERVED] {
            for w in table {
                map.insert(*w, KeywordClass::Reserved);
            }
        }
        map
    });
    if word.len() > LONGEST || !word.is_ascii() {
        return None;
    }
    let mut buf = [0u8; LONGEST];
    let upper = &mut buf[..word.len()];
    upper.copy_from_slice(word.as_bytes());
    upper.make_ascii_uppercase();
    UNION.get(std::str::from_utf8(upper).ok()?).copied()
}

/// Looks `word` up in a pair of sorted upper-case tables, ignoring case.
pub(crate) fn lookup(word: &str, reserved: &[&str], non_reserved: &[&str]) -> Option<KeywordClass> {
    if word.len() > LONGEST || !word.is_ascii() {
        return None;
    }
    let mut buf = [0u8; LONGEST];
    let upper = &mut buf[..word.len()];
    upper.copy_from_slice(word.as_bytes());
    upper.make_ascii_uppercase();
    let upper = std::str::from_utf8(upper).ok()?;
    if reserved.binary_search(&upper).is_ok() {
        Some(KeywordClass::Reserved)
    } else if non_reserved.binary_search(&upper).is_ok() {
        Some(KeywordClass::NonReserved)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_upper_case_and_disjoint() {
        let tables = [
            (POSTGRES_RESERVED, POSTGRES_NON_RESERVED),
            (ORACLE_RESERVED, ORACLE_NON_RESERVED),
            (SQLITE_RESERVED, SQLITE_NON_RESERVED),
        ];
        for (reserved, non_reserved) in tables {
            for table in [reserved, non_reserved] {
                assert!(table.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
                assert!(
                    table
                        .iter()
                        .all(|w| w.len() <= LONGEST && w.bytes().all(|b| !b.is_ascii_lowercase()))
                );
            }
            assert!(
                reserved
                    .iter()
                    .all(|w| non_reserved.binary_search(w).is_err())
            );
        }
    }
}
