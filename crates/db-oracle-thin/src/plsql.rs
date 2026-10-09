//! PL/SQL wrapper blocks.
//!
//! Every statement runs inside one anonymous block that traps errors, so a
//! failing statement returns its ORA- error as data instead of a server
//! error. That matters because oracle-rs 0.1.7 drops the connection (without
//! the error text) whenever the server reports an error during a call. Each
//! block has exactly one OUT bind, the only one oracle-rs returns reliably.
//!
//! Queries are described with `DBMS_SQL.DESCRIBE_COLUMNS3` and opened with
//! `OPEN :rc FOR <text> USING ...` (`DBMS_SQL.TO_REFCURSOR` rejects
//! `SELECT ... FOR UPDATE`). Columns that oracle-rs cannot decode are
//! converted on the server by wrapping the query as
//! `WITH PX$Q (PX$1, ...) AS (<query>) SELECT <conversions> FROM PX$Q`; a
//! top-level `FOR UPDATE` clause moves after the wrapper. A converted
//! column carries a `#PXT=<tag>` suffix in its alias (see [`split_tag`]).
//! Errors come back as a one-row cursor with the columns `PYOXIDE$ERROR` and
//! `PYOXIDE$MESSAGE`. Queries with LOB, JSON or VECTOR columns need column
//! defines, which a REF CURSOR fetch cannot do; for them the block returns
//! the (converted) statement text in a cursor with the column
//! `PYOXIDE$NATIVE`, and the driver runs it with oracle-rs directly.
//!
//! Statements without rows run with `DBMS_SQL.PARSE`/`EXECUTE`, values bound
//! by placeholder name.
//!
//! Written from the public DBMS_SQL, DBMS_UTILITY and UTL_RAW documentation.

use db_core::{Error, ErrorKind, Result};

use crate::literal::{PlsqlBind, pieces, quote};

/// Column name of the error cursor.
pub const ERROR_COLUMN: &str = "PYOXIDE$ERROR";
/// Column name of the cursor that carries a statement for the LOB path.
pub const NATIVE_COLUMN: &str = "PYOXIDE$NATIVE";
/// Separator in a converted column's alias.
pub const TAG_SEPARATOR: &str = "#PXT=";

/// Longest escaped statement piece per `VARCHAR2A` element.
const SQL_PIECE: usize = 30_000;

/// A bind value prepared for a block: the placeholder name in the user's
/// statement (`1`, `NAME`) and its PL/SQL variable.
pub struct BlockBind {
    pub placeholder: String,
    pub bind: PlsqlBind,
}

fn statement_pieces(sql: &str) -> Result<String> {
    if sql.contains('\0') {
        return Err(Error::query("the statement contains a NUL character"));
    }
    let mut out = String::new();
    for (i, p) in pieces(sql, SQL_PIECE).into_iter().enumerate() {
        out.push_str(&format!("  s({}) := {};\n", i + 1, quote(p)));
    }
    Ok(out)
}

fn decls(binds: &[BlockBind]) -> String {
    binds
        .iter()
        .enumerate()
        .map(|(i, b)| format!("  pxb{} {};\n", i + 1, b.bind.decl))
        .collect()
}

fn inits(binds: &[BlockBind]) -> String {
    binds
        .iter()
        .flat_map(|b| b.bind.init.iter().map(|s| format!("  {s}\n")))
        .collect()
}

fn bind_calls(binds: &[BlockBind]) -> String {
    binds
        .iter()
        .enumerate()
        .map(|(i, b)| {
            format!(
                "  DBMS_SQL.{}(c, {}, pxb{});\n",
                b.bind.bind_proc,
                quote(&format!(":{}", b.placeholder)),
                i + 1
            )
        })
        .collect()
}

const QUERY_BLOCK: &str = r#"DECLARE
  c INTEGER;
  cnt INTEGER;
  d DBMS_SQL.DESC_TAB3;
  s DBMS_SQL.VARCHAR2A;
  r DBMS_SQL.VARCHAR2A;
  l SYS.ODCIVARCHAR2LIST := SYS.ODCIVARCHAR2LIST();
  sq CLOB;
  fu VARCHAR2(32767);
  e VARCHAR2(32767);
  t VARCHAR2(30);
  nm VARCHAR2(4000);
  need BOOLEAN := FALSE;
  lob BOOLEAN := FALSE;
  ecode NUMBER;
  emsg VARCHAR2(4000);
/*DECLS*/
  PROCEDURE px_add(x VARCHAR2) IS BEGIN r(r.COUNT + 1) := x; END;
  PROCEDURE px_chunk(x VARCHAR2) IS
    rest VARCHAR2(32767) := x;
  BEGIN
    WHILE rest IS NOT NULL LOOP
      l.EXTEND;
      l(l.COUNT) := LPAD(TO_CHAR(l.COUNT), 6, '0') || SUBSTR(rest, 1, 900);
      rest := SUBSTR(rest, 901);
    END LOOP;
  END;
BEGIN
/*INIT*/
/*PIECES*/
/*TAIL*/
  c := DBMS_SQL.OPEN_CURSOR;
  DBMS_SQL.PARSE(c, s, 1, s.COUNT, FALSE, DBMS_SQL.NATIVE);
/*BINDS*/
  DBMS_SQL.DESCRIBE_COLUMNS3(c, cnt, d);
  DBMS_SQL.CLOSE_CURSOR(c);
  px_add('WITH PX$Q (');
  FOR i IN 1 .. cnt LOOP
    px_add(CASE WHEN i > 1 THEN ',' END || 'PX$' || i);
  END LOOP;
  px_add(') AS (' || CHR(10));
  FOR i IN 1 .. s.COUNT LOOP px_add(s(i)); END LOOP;
  px_add(CHR(10) || ') SELECT ');
  FOR i IN 1 .. cnt LOOP
    e := 'PX$' || i;
    t := NULL;
    IF d(i).col_type IN (8, 24) THEN
      RAISE_APPLICATION_ERROR(-20901, 'column ' || d(i).col_name || ': LONG and LONG RAW values cannot be fetched by the thin driver; convert them in SQL, use a *_VC dictionary column or the oracle-odpi backend');
    ELSIF d(i).col_type IN (1, 96) AND d(i).col_charsetform = 2 THEN
      e := 'TO_CHAR(' || e || ')';
      t := CASE d(i).col_type WHEN 1 THEN 'NVARCHAR2' ELSE 'NCHAR' END;
    ELSIF d(i).col_type IN (1, 96, 23) AND d(i).col_max_len = 0 THEN
      e := 'CAST(' || e || ' AS VARCHAR2(1))';
      t := 'NULL';
    ELSIF d(i).col_type = 100 THEN
      e := 'UTL_RAW.CAST_FROM_BINARY_FLOAT(' || e || ')';
      t := 'BINARY_FLOAT';
    ELSIF d(i).col_type = 101 THEN
      e := 'UTL_RAW.CAST_FROM_BINARY_DOUBLE(' || e || ')';
      t := 'BINARY_DOUBLE';
    ELSIF d(i).col_type = 182 THEN
      e := 'TO_CHAR(' || e || ')';
      t := 'INTERVAL_YM';
    ELSIF d(i).col_type = 183 THEN
      e := 'TO_CHAR(' || e || ')';
      t := 'INTERVAL_DS';
    ELSIF d(i).col_type = 181 THEN
      e := 'TO_CHAR(' || e || ', ''SYYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM'')';
      t := 'TSTZ';
    ELSIF d(i).col_type = 231 THEN
      e := 'TO_CHAR(' || e || ', ''SYYYY-MM-DD HH24:MI:SS.FF9'')';
      t := 'TSLTZ';
    ELSIF d(i).col_type = 180 AND d(i).col_scale > 6 THEN
      e := 'TO_CHAR(' || e || ', ''SYYYY-MM-DD HH24:MI:SS.FF9'')';
      t := 'TS';
    ELSIF d(i).col_type IN (11, 69) THEN
      e := 'ROWIDTOCHAR(' || e || ')';
      t := 'ROWID';
    ELSIF d(i).col_type = 208 THEN
      e := 'CAST(' || e || ' AS VARCHAR2(4000))';
      t := 'UROWID';
    ELSIF d(i).col_type = 112 AND d(i).col_charsetform = 2 THEN
      e := 'TO_CLOB(' || e || ')';
      t := 'NCLOB';
      lob := TRUE;
    ELSIF d(i).col_type IN (112, 113, 119, 127) THEN
      lob := TRUE;
    ELSIF d(i).col_type = 109 AND d(i).col_type_name = 'XMLTYPE' THEN
      e := 'XMLSERIALIZE(CONTENT ' || e || ' AS CLOB)';
      t := 'XMLTYPE';
      lob := TRUE;
    ELSIF d(i).col_type = 114 THEN
      e := 'CASE WHEN ' || e || ' IS NULL THEN NULL ELSE ''(BFILE)'' END';
      t := 'BFILE';
    ELSIF d(i).col_type IN (102, 109, 110, 111, 121, 122, 123) THEN
      e := 'CASE WHEN ' || e || ' IS NULL THEN NULL ELSE ''(' || REPLACE(d(i).col_schema_name || '.' || d(i).col_type_name, '''', '''''') || ')'' END';
      t := 'OBJECT';
    END IF;
    IF t IS NOT NULL THEN
      need := TRUE;
      nm := SUBSTRB(d(i).col_name, 1, 128 - LENGTHB(t) - 5) || '#PXT=' || t;
    ELSE
      nm := d(i).col_name;
    END IF;
    px_add(CASE WHEN i > 1 THEN ',' END || e || ' AS "' || REPLACE(nm, '"', '') || '"');
  END LOOP;
  px_add(CHR(10) || 'FROM PX$Q');
  IF NOT need THEN
    r := s;
  END IF;
  IF fu IS NOT NULL THEN
    px_add(CHR(10) || fu);
  END IF;
  IF lob THEN
    FOR i IN 1 .. r.COUNT LOOP px_chunk(r(i)); END LOOP;
    OPEN :rc FOR SELECT COLUMN_VALUE AS "PYOXIDE$NATIVE" FROM TABLE(l);
  ELSE
    FOR i IN 1 .. r.COUNT LOOP sq := sq || r(i); END LOOP;
    OPEN :rc FOR sq/*USING*/;
  END IF;
EXCEPTION WHEN OTHERS THEN
  ecode := SQLCODE;
  emsg := SUBSTR(DBMS_UTILITY.FORMAT_ERROR_STACK, 1, 4000);
  IF c IS NOT NULL THEN
    IF DBMS_SQL.IS_OPEN(c) THEN DBMS_SQL.CLOSE_CURSOR(c); END IF;
  END IF;
  OPEN :rc FOR SELECT ecode AS "PYOXIDE$ERROR", emsg AS "PYOXIDE$MESSAGE" FROM dual;
END;"#;

const EXEC_BLOCK: &str = r#"DECLARE
  c INTEGER;
  n INTEGER;
  s DBMS_SQL.VARCHAR2A;
/*DECLS*/
BEGIN
/*INIT*/
/*PIECES*/
  c := DBMS_SQL.OPEN_CURSOR;
  DBMS_SQL.PARSE(c, s, 1, s.COUNT, FALSE, DBMS_SQL.NATIVE);
/*BINDS*/
  n := DBMS_SQL.EXECUTE(c);
  DBMS_SQL.CLOSE_CURSOR(c);
/*COMMIT*/
  :o := '0' || CHR(31) || TO_CHAR(n);
EXCEPTION WHEN OTHERS THEN
  :o := TO_CHAR(SQLCODE) || CHR(31) || SUBSTR(DBMS_UTILITY.FORMAT_ERROR_STACK, 1, 4000);
  IF c IS NOT NULL THEN
    IF DBMS_SQL.IS_OPEN(c) THEN DBMS_SQL.CLOSE_CURSOR(c); END IF;
  END IF;
END;"#;

const PROBE_BLOCK: &str = r#"DECLARE
  ok VARCHAR2(1) := 'N';
  oc VARCHAR2(100);
  v VARCHAR2(4000);
  iv BINARY_INTEGER;
  sv VARCHAR2(4000);
  tp BINARY_INTEGER;
BEGIN
  BEGIN
    EXECUTE IMMEDIATE 'DECLARE c INTEGER; BEGIN c := DBMS_SQL.OPEN_CURSOR; DBMS_SQL.CLOSE_CURSOR(c); END;';
    ok := 'Y';
  EXCEPTION WHEN OTHERS THEN ok := 'N';
  END;
  BEGIN
    EXECUTE IMMEDIATE 'SELECT value FROM v$parameter WHERE name = ''open_cursors''' INTO oc;
  EXCEPTION WHEN OTHERS THEN
    BEGIN
      EXECUTE IMMEDIATE 'BEGIN :a := DBMS_UTILITY.GET_PARAMETER_VALUE(''open_cursors'', :b, :c); END;' USING OUT tp, OUT iv, OUT sv;
      oc := TO_CHAR(iv);
    EXCEPTION WHEN OTHERS THEN oc := NULL;
    END;
  END;
  BEGIN
    EXECUTE IMMEDIATE 'SELECT banner_full FROM v$version WHERE banner LIKE ''Oracle%'' AND ROWNUM = 1' INTO v;
  EXCEPTION WHEN OTHERS THEN
    BEGIN
      EXECUTE IMMEDIATE 'SELECT banner FROM v$version WHERE banner LIKE ''Oracle%'' AND ROWNUM = 1' INTO v;
    EXCEPTION WHEN OTHERS THEN
      BEGIN
        EXECUTE IMMEDIATE 'SELECT product || '' '' || version_full FROM product_component_version WHERE product LIKE ''Oracle%'' AND ROWNUM = 1' INTO v;
      EXCEPTION WHEN OTHERS THEN
        BEGIN
          EXECUTE IMMEDIATE 'SELECT product || '' '' || version FROM product_component_version WHERE product LIKE ''Oracle%'' AND ROWNUM = 1' INTO v;
        EXCEPTION WHEN OTHERS THEN v := NULL;
        END;
      END;
    END;
  END;
  OPEN :rc FOR SELECT ok AS ok, oc AS oc, v AS banner, RPAD(TO_CHAR(LEVEL), /*PAD*/, 'x') AS pad FROM dual CONNECT BY LEVEL <= /*ROWS*/;
END;"#;

/// The block that opens a query (see the module docs). `body` is the query
/// without a top-level `FOR UPDATE` clause, which is passed as `tail`.
/// `occurrences` lists, for every placeholder occurrence in `body` in order,
/// the index of its bind in `binds` (`OPEN ... USING` binds by position).
pub fn query_block(
    body: &str,
    tail: Option<&str>,
    binds: &[BlockBind],
    occurrences: &[usize],
) -> Result<String> {
    let tail = match tail {
        Some(t) => format!("  fu := {};\n", quote(t)),
        None => String::new(),
    };
    let using = if occurrences.is_empty() {
        String::new()
    } else {
        format!(
            " USING {}",
            occurrences
                .iter()
                .map(|i| format!("pxb{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Ok(QUERY_BLOCK
        .replace("/*DECLS*/\n", &decls(binds))
        .replace("/*BINDS*/\n", &bind_calls(binds))
        .replace("/*INIT*/\n", &inits(binds))
        .replace("/*PIECES*/\n", &statement_pieces(body)?)
        .replace("/*TAIL*/\n", &tail)
        .replace("/*USING*/", &using))
}

/// The block that runs a statement without rows; its OUT value is parsed by
/// [`parse_exec_status`]. `commit` adds a COMMIT after success.
pub fn exec_block(sql: &str, binds: &[BlockBind], commit: bool) -> Result<String> {
    Ok(EXEC_BLOCK
        .replace("/*DECLS*/\n", &decls(binds))
        .replace("/*BINDS*/\n", &bind_calls(binds))
        .replace("/*INIT*/\n", &inits(binds))
        .replace("/*PIECES*/\n", &statement_pieces(sql)?)
        .replace("/*COMMIT*/\n", if commit { "  COMMIT;\n" } else { "" }))
}

/// The session probe run after connecting: whether `DBMS_SQL` is usable,
/// `OPEN_CURSORS`, the version banner, and `rows` rows of `pad` bytes to
/// test how large a single response packet may be.
pub fn probe_block(rows: u32, pad: u32) -> String {
    PROBE_BLOCK
        .replace("/*PAD*/", &pad.max(1).to_string())
        .replace("/*ROWS*/", &rows.max(1).to_string())
}

/// An [`Error`] for an Oracle error trapped by a wrapper block.
pub fn trapped_error(sqlcode: i64, stack: &str) -> Error {
    // Frames of the wrapper's own DBMS_SQL calls are noise for the user.
    let message = stack
        .lines()
        .filter(|l| !l.contains("at \"SYS.DBMS_SQL\""))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned();
    let mut e = db_core::oracle::error_from_message(message, ErrorKind::Query);
    if e.code.is_none() && sqlcode != 0 {
        let code = if sqlcode == 100 { 1403 } else { sqlcode.abs() };
        let code = format!("ORA-{code:05}");
        e.kind = db_core::oracle::classify(&code);
        e.code = Some(code);
    }
    e
}

/// Rows affected, or the trapped error, from an exec block's OUT value.
pub fn parse_exec_status(s: &str) -> Result<u64> {
    let (code, rest) = s
        .split_once('\u{1f}')
        .ok_or_else(|| Error::internal(format!("unexpected wrapper result `{s}`")))?;
    let code: i64 = code
        .trim()
        .parse()
        .map_err(|_| Error::internal(format!("unexpected wrapper result `{s}`")))?;
    if code == 0 {
        Ok(rest.trim().parse().unwrap_or(0))
    } else {
        Err(trapped_error(code, rest))
    }
}

/// Split a converted column's alias into the user's name and the tag.
pub fn split_tag(name: &str) -> (&str, Option<&str>) {
    match name.rfind(TAG_SEPARATOR) {
        Some(p) => (&name[..p], Some(&name[p + TAG_SEPARATOR.len()..])),
        None => (name, None),
    }
}

/// Reassemble the statement text from the `PYOXIDE$NATIVE` rows
/// (`NNNNNN` sequence prefix + up to 900 characters each).
pub fn join_native_chunks(rows: &[String]) -> String {
    let mut parts: Vec<(u32, &str)> = rows
        .iter()
        .map(|r| {
            let (n, text) = r.split_at(r.len().min(6));
            (n.parse().unwrap_or(u32::MAX), text)
        })
        .collect();
    parts.sort_by_key(|(n, _)| *n);
    parts.into_iter().map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literal::plsql_bind;
    use db_core::Value;

    fn binds() -> Vec<BlockBind> {
        vec![
            BlockBind {
                placeholder: "1".into(),
                bind: plsql_bind("pxb1", &Value::Int(7)).unwrap(),
            },
            BlockBind {
                placeholder: "NAME".into(),
                bind: plsql_bind("pxb2", &Value::Bytes(vec![1])).unwrap(),
            },
        ]
    }

    fn outer_binds(block: &str) -> Vec<String> {
        crate::sql::placeholders(block)
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    #[test]
    fn builds_query_block() {
        let b = query_block(
            "SELECT :1, :name, :1 FROM t WHERE x = 'it''s'",
            Some("FOR UPDATE"),
            &binds(),
            &[0, 1, 0],
        )
        .unwrap();
        assert!(b.contains("  pxb1 NUMBER := 7;\n"), "{b}");
        assert!(b.contains("OPEN :rc FOR sq USING pxb1, pxb2, pxb1;"), "{b}");
        assert!(b.contains("  fu := 'FOR UPDATE';\n"), "{b}");
        assert!(
            b.contains("s(1) := 'SELECT :1, :name, :1 FROM t WHERE x = ''it''''s''';"),
            "{b}"
        );
        assert!(!b.contains("/*"), "unreplaced marker in {b}");
        // The only bind of the block itself is the OUT cursor.
        assert!(outer_binds(&b).iter().all(|n| n == "RC"));
        let plain = query_block("SELECT 1 FROM dual", None, &[], &[]).unwrap();
        assert!(plain.contains("OPEN :rc FOR sq;"), "{plain}");
    }

    #[test]
    fn builds_exec_block() {
        let b = exec_block("UPDATE t SET a = :1", &binds()[..1], true).unwrap();
        assert!(b.contains("  COMMIT;\n"));
        assert!(b.contains("  DBMS_SQL.BIND_VARIABLE(c, ':1', pxb1);"));
        let b = exec_block("UPDATE t SET a = 1", &[], false).unwrap();
        assert!(!b.contains("COMMIT"));
        assert!(!b.contains("/*"));
        assert!(outer_binds(&b).iter().all(|n| n == "O"));
        assert!(exec_block("x\0y", &[], false).is_err());
    }

    #[test]
    fn long_statements_are_split() {
        let sql = format!("SELECT '{}' FROM dual", "z".repeat(70_000));
        let b = exec_block(&sql, &[], false).unwrap();
        assert!(b.contains("s(3) := "));
    }

    #[test]
    fn probe_block_only_binds_rc() {
        let b = probe_block(12, 4000);
        assert!(b.contains("RPAD(TO_CHAR(LEVEL), 4000, 'x')"));
        assert!(b.contains("LEVEL <= 12;"));
        assert!(outer_binds(&b).iter().all(|n| n == "RC"));
    }

    #[test]
    fn parses_exec_status() {
        assert_eq!(parse_exec_status("0\u{1f}3").unwrap(), 3);
        let e = parse_exec_status(
            "-942\u{1f}ORA-00942: table or view does not exist\nORA-06512: at \"SYS.DBMS_SQL\", line 1386\n",
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query);
        assert_eq!(e.code.as_deref(), Some("ORA-00942"));
        assert_eq!(e.message, "ORA-00942: table or view does not exist");
        let e = parse_exec_status("-1017\u{1f}no code in text").unwrap_err();
        assert_eq!(e.code.as_deref(), Some("ORA-01017"));
        assert_eq!(e.kind, ErrorKind::Auth);
        assert!(parse_exec_status("garbage").is_err());
    }

    #[test]
    fn splits_tags_and_chunks() {
        assert_eq!(
            split_tag("AMOUNT#PXT=BINARY_DOUBLE"),
            ("AMOUNT", Some("BINARY_DOUBLE"))
        );
        assert_eq!(split_tag("PLAIN"), ("PLAIN", None));
        let rows = vec!["000002world".to_string(), "000001hello ".to_string()];
        assert_eq!(join_native_chunks(&rows), "hello world");
    }
}
