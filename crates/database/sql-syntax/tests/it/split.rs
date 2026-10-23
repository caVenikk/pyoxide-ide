use sql_syntax::{Dialect, split, statement_at, statements_in};

use crate::{check_ranges, texts};

const PG: Dialect = Dialect::Postgres;
const ORA: Dialect = Dialect::Oracle;
const LITE: Dialect = Dialect::Sqlite;

#[test]
fn basic_split_and_ranges() {
    let s = "  SELECT 1 ;\n\nselect 2;select 3  ";
    for d in Dialect::ALL {
        let st = split(s, d);
        check_ranges(s, &st);
        assert_eq!(st.len(), 3);
        assert_eq!(st[0].text, "SELECT 1");
        assert_eq!(st[0].range.start, 2);
        assert_eq!(st[1].text, "select 2");
        assert_eq!(st[2].text, "select 3");
        assert_eq!(st[2].range.end, s.len() - 2);
    }
}

#[test]
fn semicolons_in_strings_comments_and_quoted_names_do_not_split() {
    let s = "SELECT 'a;b', \"x;y\" -- c;d\nFROM t /* e;f */ WHERE x = 'it''s;'; SELECT 2";
    for d in Dialect::ALL {
        let t = texts(s, d);
        assert_eq!(t.len(), 2, "{d:?}");
        assert!(t[0].ends_with("'it''s;'"));
        assert_eq!(t[1], "SELECT 2");
    }
    assert_eq!(texts("SELECT [a;b] FROM `c;d`; SELECT 1", LITE).len(), 2);
}

#[test]
fn nested_comments_on_postgres() {
    assert_eq!(texts("SELECT 1 /* a /* b; */ c; */; SELECT 2", PG).len(), 2);
    // Not nested elsewhere: the first `*/` closes the comment.
    assert_eq!(
        texts("SELECT 1 /* a /* b; */ c; */; SELECT 2", LITE).len(),
        3
    );
}

#[test]
fn dollar_quoting() {
    let s = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN; RETURN 1; END; $$ LANGUAGE plpgsql;\n\
             DO $body$ BEGIN PERFORM 1; END $body$; SELECT $1";
    let t = texts(s, PG);
    assert_eq!(t.len(), 3, "{t:?}");
    assert!(t[0].ends_with("LANGUAGE plpgsql"));
    assert!(t[1].starts_with("DO $body$"));
    assert_eq!(t[2], "SELECT $1");
    // Nested dollar quotes with another tag.
    assert_eq!(texts("select $a$ x $$ ; $$ y $a$; select 2", PG).len(), 2);
}

#[test]
fn postgres_begin_atomic_bodies() {
    let s = "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END;\nSELECT 3; BEGIN; SELECT 4; COMMIT";
    let t = texts(s, PG);
    assert_eq!(t.len(), 5, "{t:?}");
    assert!(t[0].ends_with("END"));
    assert_eq!(t[1], "SELECT 3");
    assert_eq!(&t[2..], ["BEGIN", "SELECT 4", "COMMIT"]);
}

#[test]
fn empty_and_comment_only() {
    for d in Dialect::ALL {
        assert!(texts("", d).is_empty());
        assert!(texts(" ;; \n ; ", d).is_empty());
        assert!(texts("-- only\n/* comment */;", d).is_empty());
        assert_eq!(texts("-- header\nSELECT 1; -- tail\n", d), ["SELECT 1"]);
    }
}

#[test]
fn sqlite_trigger_bodies() {
    let trig = "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = CASE WHEN 1 THEN 2 END; DELETE FROM b; END;\nSELECT 1;";
    let t = texts(trig, LITE);
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t[0].ends_with("END"));
    assert_eq!(t[1], "SELECT 1");
    assert_eq!(texts("BEGIN; SELECT 1; COMMIT", LITE).len(), 3);
    assert_eq!(
        texts(
            "CREATE TEMP TRIGGER t AFTER INSERT ON a BEGIN SELECT 1; END; SELECT 2",
            LITE
        )
        .len(),
        2
    );
}

#[test]
fn unclosed_constructs_do_not_panic_or_split() {
    for d in Dialect::ALL {
        for s in [
            "SELECT 'abc",
            "SELECT \"abc",
            "SELECT /* abc",
            "SELECT $$ abc",
            "SELECT $tag$ abc $ta",
            "SELECT [abc",
            "[",
            "[x]",
            "[a] FROM t",
            "SELECT `abc",
            "SELECT E'abc\\",
            "SELECT E'\\ñ",
            "1e+",
            "'",
            "$",
            "SELECT q'[abc; def",
            "select q'",
        ] {
            let st = split(s, d);
            check_ranges(s, &st);
            assert_eq!(st.len(), 1, "{s} {d:?}");
            let _ = statement_at(s, 3, d);
        }
    }
    assert_eq!(texts("SELECT 'a; SELECT 2", PG).len(), 1);
    assert!(texts("--", PG).is_empty() && texts("/*", PG).is_empty());
}

#[test]
fn unicode_text() {
    let s = "SELECT 'canción 🎵;' AS título; -- ñandú 🦤\nSELECT \"año\" FROM café;";
    for d in Dialect::ALL {
        let st = split(s, d);
        assert_eq!(st.len(), 2);
        assert_eq!(st[1].text, "SELECT \"año\" FROM café");
        check_ranges(s, &st);
        // Any position, even in the middle of a multibyte character.
        for c in 0..=s.len() + 5 {
            assert!(statement_at(s, c, d).is_some());
        }
    }
}

#[test]
fn statement_at_positions() {
    let s = "SELECT 1;\n\n  SELECT 2;   \n\nSELECT 3";
    let at = |c| statement_at(s, c, PG).unwrap().text;
    assert_eq!(at(0), "SELECT 1");
    assert_eq!(at(3), "SELECT 1");
    assert_eq!(at(8), "SELECT 1"); // right before the `;`
    assert_eq!(at(9), "SELECT 1"); // after the `;`
    assert_eq!(at(10), "SELECT 1"); // the blank line
    assert_eq!(at(11), "SELECT 2"); // the indentation of the SELECT 2 line
    let p2 = s.find("SELECT 2").unwrap();
    assert_eq!(at(p2 + 4), "SELECT 2");
    assert_eq!(at(p2 + 12), "SELECT 2"); // spaces after the `;`
    assert_eq!(at(s.len() - 12), "SELECT 2"); // the blank line before the last one
    assert_eq!(at(s.len()), "SELECT 3");
    assert_eq!(at(s.len() + 100), "SELECT 3");

    let s2 = "\n\n  -- hello\n SELECT 1; SELECT 2";
    assert_eq!(statement_at(s2, 0, PG).unwrap().text, "SELECT 1");
    let st = statement_at(s2, s2.len(), PG).unwrap();
    assert_eq!(st.range, s2.len() - 8..s2.len());
    assert_eq!(statement_at("  -- nothing\n", 3, PG), None);
    assert_eq!(statement_at("", 0, PG), None);
}

#[test]
fn statements_in_a_selection() {
    let s = "select 1; select 22; select 333;";
    let r = s.find("select 22").unwrap();
    let st = statements_in(s, r..s.len(), PG);
    check_ranges(s, &st);
    assert_eq!(
        st.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
        ["select 22", "select 333"]
    );
    // A selection that cuts a statement runs the cut text.
    let st = statements_in(s, 0..r + 6, PG);
    assert_eq!(
        st.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(),
        ["select 1", "select"]
    );
    // Ends are clamped and moved onto character boundaries.
    let u = "select 'é'; select 2";
    let st = statements_in(u, 0..u.find('é').unwrap() + 1, PG);
    assert_eq!(st.len(), 1);
    assert!(statements_in(u, std::ops::Range { start: 5, end: 2 }, PG).is_empty());
    assert_eq!(statements_in(u, 0..1000, PG).len(), 2);
}

#[test]
fn oracle_plsql_blocks() {
    let d = ORA;
    // Without `/`: up to the `;` of the outermost END, which is kept.
    let s = "BEGIN\n  IF x THEN y; END IF;\n  FOR i IN 1..3 LOOP z; END LOOP;\n  CASE a WHEN 1 THEN b; END CASE;\nEND;\nselect 1 from dual;";
    let st = split(s, d);
    check_ranges(s, &st);
    assert_eq!(st.len(), 2, "{st:?}");
    assert!(st[0].text.starts_with("BEGIN") && st[0].text.ends_with("END;"));
    assert_eq!(st[1].text, "select 1 from dual");
    let s = "DECLARE\n  n NUMBER := 1;\nBEGIN\n  BEGIN NULL; END;\n  n := 2;\nEND my_block;\nselect 2 from dual";
    let t = texts(s, d);
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t[0].ends_with("END my_block;"));
    let s = "<<outer>> BEGIN\n  NULL;\nEND;\nselect 2 from dual";
    assert_eq!(texts(s, d).len(), 2);
    let s = "CREATE OR REPLACE EDITIONABLE PROCEDURE p(a NUMBER) IS\n  v NUMBER;\n  CURSOR c IS SELECT 1 FROM dual;\nBEGIN\n  v := CASE WHEN a > 1 THEN 1 ELSE 2 END;\nEND p;\nSELECT 3 FROM dual;";
    let t = texts(s, d);
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t[0].ends_with("END p;"));
    let s = "CREATE OR REPLACE PACKAGE BODY pk AS\n  PROCEDURE a IS BEGIN NULL; END a;\n  FUNCTION b RETURN NUMBER IS BEGIN RETURN 1; END;\nBEGIN\n  NULL;\nEND pk;\nSELECT 4 FROM dual";
    let t = texts(s, d);
    assert_eq!(t.len(), 2, "{t:?}");
    assert!(t[0].ends_with("END pk;"));
    let s = "CREATE PACKAGE pk AS\n  PROCEDURE a;\n  FUNCTION b RETURN NUMBER;\nEND pk;\nSELECT 5 FROM dual";
    assert_eq!(texts(s, d).len(), 2);
    let s = "CREATE OR REPLACE TRIGGER trg BEFORE INSERT ON t REFERENCING NEW AS n FOR EACH ROW\nBEGIN\n  :n.x := 1;\nEND;\nSELECT 6 FROM dual";
    assert_eq!(texts(s, d).len(), 2);
    let s = "CREATE TYPE BODY ty AS\n  MEMBER FUNCTION f RETURN NUMBER IS BEGIN RETURN 1; END;\nEND;\nSELECT 7 FROM dual";
    assert_eq!(texts(s, d).len(), 2);
    let s = "CREATE TYPE ty AS OBJECT (a NUMBER);\nSELECT 8 FROM dual";
    assert_eq!(
        texts(s, d),
        ["CREATE TYPE ty AS OBJECT (a NUMBER);", "SELECT 8 FROM dual"]
    );
    let s = "CREATE FUNCTION j RETURN NUMBER AS LANGUAGE JAVA NAME 'X.y() return int';\nSELECT 9 FROM dual";
    assert_eq!(texts(s, d).len(), 2);
    // With `/`: up to that line, whatever the nesting looks like.
    let s = "begin\n  x;\nend;\n/\nselect 1 from dual\n/\n  /\nDECLARE n NUMBER; BEGIN IF a THEN b; END IF; END;\n/";
    let st = split(s, d);
    check_ranges(s, &st);
    let t: Vec<_> = st.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(
        t,
        [
            "begin\n  x;\nend;",
            "select 1 from dual",
            "DECLARE n NUMBER; BEGIN IF a THEN b; END IF; END;"
        ]
    );
    // Plain SQL is split by `;`; a `/` in an expression is a division.
    assert_eq!(
        texts("select 4 / 2 from dual; select 1 from dual", d).len(),
        2
    );
    assert_eq!(texts("select 4\n/ 2 from dual", d).len(), 1);
    // The block under the caret is the whole block.
    let s = "select 1 from dual;\nBEGIN\n  a;\n  b;\nEND;\n/\nselect 2 from dual;";
    let st = statement_at(s, s.find("b;").unwrap(), d).unwrap();
    assert_eq!(st.text, "BEGIN\n  a;\n  b;\nEND;");
    assert_eq!(&s[st.range.clone()], st.text);
    assert_eq!(split(s, d).len(), 3);
    // Other engines have no `/` line and no bare blocks.
    assert_eq!(texts("BEGIN; SELECT 1; COMMIT", PG).len(), 3);
    assert_eq!(texts("BEGIN; SELECT 1; COMMIT", Dialect::Generic).len(), 3);
    assert_eq!(texts("select 1\n/\nselect 2", PG).len(), 1);
}

#[test]
fn generic_dialect_keeps_trigger_and_routine_bodies() {
    let s = "CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE a SET x = 1; END;\nSELECT 1;";
    assert_eq!(texts(s, Dialect::Generic).len(), 2);
    let s = "CREATE PROCEDURE p() BEGIN IF a THEN SELECT 1; END IF; WHILE b DO SELECT 2; END WHILE; END;\nSELECT 3";
    assert_eq!(texts(s, Dialect::Generic).len(), 2);
}

#[test]
fn oracle_q_quotes() {
    for s in [
        "select q'[it's; x]' from dual; select 2 from dual",
        "select Q'{a ' ; }' from dual; select 2 from dual",
        "select q'(a ';' )' from dual; select 2 from dual",
        "select q'<a;>' from dual; select 2 from dual",
        "select q'!a;'!' from dual; select 2 from dual",
        "select nq'[ñ;]' from dual; select 2 from dual",
        "select NQ'#x;#' from dual; select 2 from dual",
    ] {
        let st = split(s, ORA);
        check_ranges(s, &st);
        assert_eq!(st.len(), 2, "{s}");
    }
    // Elsewhere `q` is a name.
    assert_eq!(texts("select q'x'; select 2", PG).len(), 2);
    assert_eq!(texts("select q 'a;' from t", ORA).len(), 1);
}
