use sql_syntax::{Dialect, KeywordClass};

const PG: Dialect = Dialect::Postgres;
const ORA: Dialect = Dialect::Oracle;
const LITE: Dialect = Dialect::Sqlite;

#[test]
fn ids_round_trip() {
    for d in Dialect::ALL {
        assert_eq!(Dialect::from_id(d.id()), Some(d));
    }
    assert_eq!(Dialect::from_id("PostgreSQL"), None);
    assert_eq!(Dialect::from_id("Oracle"), Some(ORA));
    assert_eq!(Dialect::default(), Dialect::Generic);
}

#[test]
fn folding_unquoted_names() {
    assert_eq!(PG.fold("Customers"), "customers");
    assert_eq!(ORA.fold("Customers"), "CUSTOMERS");
    assert_eq!(LITE.fold("Customers"), "customers");
    assert_eq!(Dialect::Generic.fold("Customers"), "customers");
    assert_eq!(ORA.fold("straße"), "STRASSE");
    // PostgreSQL folds ASCII letters only.
    assert_eq!(PG.fold("ÄB"), "Äb");
}

#[test]
fn quoted_names_are_kept() {
    assert_eq!(PG.fold("\"Customers\""), "Customers");
    assert_eq!(ORA.fold("\"Customers\""), "Customers");
    assert_eq!(LITE.fold("\"Customers\""), "Customers");
    assert_eq!(PG.fold("\"a\"\"b\""), "a\"b");
    assert_eq!(LITE.fold("`Mixed Case`"), "Mixed Case");
    assert_eq!(LITE.fold("[Mixed Case]"), "Mixed Case");
    // Not quotes in PostgreSQL and Oracle.
    assert_eq!(PG.fold("`X`"), "`x`");
    // An unterminated quote reads to the end.
    assert_eq!(PG.fold("\"abc"), "abc");
    // The folded forms of one object agree.
    assert_eq!(PG.fold("FOO"), PG.fold("\"foo\""));
    assert_eq!(ORA.fold("foo"), ORA.fold("\"FOO\""));
    assert_ne!(PG.fold("\"Foo\""), PG.fold("foo"));
}

#[test]
fn sqlite_compares_names_case_insensitively() {
    assert!(LITE.names_equal(&LITE.fold("Users"), &LITE.fold("\"uSERS\"")));
    assert!(LITE.names_equal("Users", "users"));
    assert!(!PG.names_equal("Users", "users"));
    assert!(!ORA.names_equal("Users", "users"));
    assert!(PG.names_equal("users", "users"));
    // ASCII only.
    assert!(!LITE.names_equal("É", "é"));
}

#[test]
fn quoting_catalog_names() {
    assert_eq!(PG.quote("orders"), "orders");
    assert_eq!(PG.quote("Orders"), "\"Orders\"");
    assert_eq!(PG.quote("my table"), "\"my table\"");
    assert_eq!(PG.quote("select"), "\"select\"");
    assert_eq!(PG.quote("user"), "\"user\"");
    assert_eq!(PG.quote("a\"b"), "\"a\"\"b\"");
    assert_eq!(PG.quote(""), "\"\"");
    assert_eq!(PG.quote("1abc"), "\"1abc\"");
    assert_eq!(PG.quote("a$b"), "a$b");
    assert_eq!(ORA.quote("ORDERS"), "ORDERS");
    assert_eq!(ORA.quote("orders"), "\"orders\"");
    assert_eq!(ORA.quote("ORDER"), "\"ORDER\"");
    assert_eq!(ORA.quote("A#1"), "A#1");
    assert_eq!(LITE.quote("Orders"), "Orders");
    assert_eq!(LITE.quote("order"), "\"order\"");
    assert_eq!(LITE.quote("a-b"), "\"a-b\"");
    assert_eq!(Dialect::Generic.quote("orders"), "orders");
    assert_eq!(Dialect::Generic.quote("Orders"), "\"Orders\"");
    // A quoted name folds back to what it was made from (SQLite: up to case).
    for d in Dialect::ALL {
        for n in ["orders", "ORDERS", "Orders", "a b", "select", "ü", "x\"y"] {
            assert!(d.names_equal(&d.fold(&d.quote(n)), n), "{d:?} {n}");
        }
    }
}

#[test]
fn keyword_tables() {
    use KeywordClass::*;
    assert_eq!(PG.keyword("select"), Some(Reserved));
    assert_eq!(PG.keyword("SeLeCt"), Some(Reserved));
    assert_eq!(PG.keyword("user"), Some(Reserved));
    assert_eq!(PG.keyword("ilike"), Some(Reserved));
    assert_eq!(PG.keyword("returning"), Some(Reserved));
    assert_eq!(PG.keyword("name"), Some(NonReserved));
    assert_eq!(PG.keyword("customers"), None);
    assert_eq!(PG.keyword("é"), None);
    assert_eq!(PG.keyword(&"a".repeat(100)), None);
    assert_eq!(ORA.keyword("select"), Some(Reserved));
    assert_eq!(ORA.keyword("rownum"), Some(Reserved));
    assert_eq!(ORA.keyword("varchar2"), Some(Reserved));
    assert_eq!(ORA.keyword("limit"), Some(NonReserved));
    assert_eq!(ORA.keyword("ilike"), None);
    assert_eq!(LITE.keyword("select"), Some(Reserved));
    assert_eq!(LITE.keyword("pragma"), Some(NonReserved));
    assert_eq!(LITE.keyword("autoincrement"), Some(Reserved));
    assert_eq!(LITE.keyword("rownum"), None);
    // Generic: reserved wherever any dialect reserves it.
    assert_eq!(Dialect::Generic.keyword("rownum"), Some(Reserved));
    assert_eq!(Dialect::Generic.keyword("ilike"), Some(Reserved));
    assert_eq!(Dialect::Generic.keyword("customers"), None);
    assert!(PG.is_reserved("from") && !PG.is_reserved("name"));
}

#[test]
fn parameter_defaults() {
    assert!(PG.default_params().dollar_numbered && !LITE.default_params().dollar_numbered);
    assert!(ORA.default_params().colon_numbered);
    assert!(LITE.default_params().at_named);
    assert!(!PG.default_params().pyformat);
}
