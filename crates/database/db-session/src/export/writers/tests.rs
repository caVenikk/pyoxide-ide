use chrono::NaiveDate;
use db_core::{Column, Dialect, Value, ValueKind};

use super::*;

fn col(name: &str, kind: ValueKind) -> Column {
    Column {
        name: name.into(),
        type_name: "x".into(),
        kind,
        nullable: true,
    }
}

fn columns() -> Vec<Column> {
    vec![
        col("id", ValueKind::Int),
        col("name", ValueKind::Text),
        col("note", ValueKind::Text),
    ]
}

/// Commas, quotes, line breaks, NULL, an empty text, unicode.
fn rows() -> Vec<Vec<Value>> {
    vec![
        vec![Value::Int(1), Value::Text("Ada, L.".into()), Value::Null],
        vec![
            Value::Int(2),
            Value::Text("Line\nbreak".into()),
            Value::Text("say \"hi\" now".into()),
        ],
        vec![
            Value::Int(3),
            Value::Text("Grace Хоппер 日本".into()),
            Value::Text(String::new()),
        ],
    ]
}

fn options(format: Format) -> ExportOptions {
    ExportOptions::new(format, Dialect::Postgres)
}

fn out(options: &ExportOptions) -> String {
    to_string(options, &columns(), &rows())
}

#[test]
fn csv_quotes_what_needs_it() {
    let o = ExportOptions {
        column_header: true,
        ..options(Format::Csv)
    };
    assert_eq!(
        out(&o),
        "id,name,note\n\
         1,\"Ada, L.\",\n\
         2,\"Line\nbreak\",\"say \"\"hi\"\" now\"\n\
         3,Grace Хоппер 日本,\"\"\n"
    );
}

#[test]
fn csv_without_a_header_and_with_a_null_text() {
    let o = ExportOptions {
        null_text: "\\N".into(),
        ..options(Format::Csv)
    };
    let text = out(&o);
    assert!(text.starts_with("1,\"Ada, L.\",\\N\n"), "{text}");
    // An empty text stays distinguishable from NULL.
    assert!(text.ends_with("3,Grace Хоппер 日本,\"\"\n"), "{text}");
    // A text that is the NULL text is quoted.
    let rows = vec![vec![Value::Int(1), Value::Text("\\N".into()), Value::Null]];
    assert_eq!(to_string(&o, &columns(), &rows), "1,\"\\N\",\\N\n");
}

#[test]
fn other_delimiters() {
    let rows = vec![vec![
        Value::Int(1),
        Value::Text("a;b|c\td".into()),
        Value::Null,
    ]];
    for (format, expected) in [
        (Format::Tsv, "1\t\"a;b|c\td\"\t\n"),
        (Format::PipeSeparated, "1|\"a;b|c\td\"|\n"),
        (Format::SemicolonSeparated, "1;\"a;b|c\td\";\n"),
    ] {
        assert_eq!(to_string(&options(format), &columns(), &rows), expected);
    }
}

#[test]
fn always_quote_keeps_nulls_bare() {
    let o = ExportOptions {
        always_quote: true,
        ..options(Format::Csv)
    };
    let rows = vec![vec![Value::Int(1), Value::Text("a".into()), Value::Null]];
    assert_eq!(to_string(&o, &columns(), &rows), "\"1\",\"a\",\n");
}

#[test]
fn row_header_numbers_the_rows() {
    let o = ExportOptions {
        column_header: true,
        row_header: true,
        ..options(Format::Csv)
    };
    let text = out(&o);
    assert!(text.starts_with(",id,name,note\n1,1,"), "{text}");
    assert!(text.contains("\n3,3,Grace"), "{text}");
}

#[test]
fn transposed_csv_turns_columns_into_lines() {
    let o = ExportOptions {
        column_header: true,
        transpose: true,
        ..options(Format::Csv)
    };
    let rows = vec![
        vec![Value::Int(1), Value::Text("a".into()), Value::Null],
        vec![
            Value::Int(2),
            Value::Text("b".into()),
            Value::Text("c".into()),
        ],
    ];
    assert_eq!(
        to_string(&o, &columns(), &rows),
        "id,1,2\nname,a,b\nnote,,c\n"
    );
}

#[test]
fn markdown_escapes_pipes_and_breaks_and_aligns_numbers() {
    let o = options(Format::Markdown);
    let rows = vec![vec![
        Value::Int(1),
        Value::Text("a|b\nc".into()),
        Value::Null,
    ]];
    assert_eq!(
        to_string(&o, &columns(), &rows),
        "| id | name | note |\n| ---: | --- | --- |\n| 1 | a\\|b<br>c |  |\n"
    );
}

#[test]
fn html_escapes_markup() {
    let rows = vec![vec![
        Value::Int(1),
        Value::Text("<b>&\"x\"</b>".into()),
        Value::Null,
    ]];
    let text = to_string(&options(Format::Html), &columns(), &rows);
    assert!(text.contains("<table>"), "{text}");
    assert!(text.contains("<th>name</th>"), "{text}");
    assert!(
        text.contains("<td>&lt;b&gt;&amp;&quot;x&quot;&lt;/b&gt;</td>"),
        "{text}"
    );
    assert!(text.trim_end().ends_with("</html>"), "{text}");
}

#[test]
fn json_is_typed_and_parses_back() {
    let cols = vec![
        col("id", ValueKind::Int),
        col("name", ValueKind::Text),
        col("price", ValueKind::Decimal),
        col("doc", ValueKind::Json),
        col("raw", ValueKind::Bytes),
        col("day", ValueKind::Date),
        col("ok", ValueKind::Bool),
    ];
    let rows = vec![
        vec![
            Value::Int(1),
            Value::Text("a \"q\"\nline, 日本".into()),
            Value::Decimal("10.50".into()),
            Value::Json("{\"n\": [1, 2]}".into()),
            Value::Bytes(vec![0xca, 0xfe]),
            Value::Date(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()),
            Value::Bool(true),
        ],
        vec![
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ],
    ];
    let text = to_string(&options(Format::Json), &cols, &rows);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    let first = &parsed[0];
    assert_eq!(first["id"], 1);
    assert_eq!(first["name"], "a \"q\"\nline, 日本");
    assert!(text.contains("\"price\": 10.50,"), "{text}");
    assert_eq!(first["doc"]["n"][1], 2);
    assert_eq!(first["raw"], "0xCAFE");
    assert_eq!(first["day"], "2026-09-01");
    assert_eq!(first["ok"], true);
    assert!(parsed[1]["id"].is_null());
    assert_eq!(to_string(&options(Format::Json), &cols, &[]), "[]\n");
}

#[test]
fn sql_inserts_carry_the_table_and_escape() {
    let o = ExportOptions {
        table: Some("demo.customers".into()),
        ..options(Format::SqlInserts)
    };
    assert_eq!(
        out(&o),
        "INSERT INTO demo.customers (id, name, note) VALUES (1, 'Ada, L.', NULL);\n\
         INSERT INTO demo.customers (id, name, note) VALUES (2, 'Line\nbreak', 'say \"hi\" now');\n\
         INSERT INTO demo.customers (id, name, note) VALUES (3, 'Grace Хоппер 日本', '');\n"
    );
}

#[test]
fn sql_updates_use_the_keys_or_every_column() {
    let o = ExportOptions {
        table: Some("t".into()),
        key_columns: vec!["id".into()],
        ..options(Format::SqlUpdates)
    };
    let text = out(&o);
    assert!(
        text.starts_with("UPDATE t SET name = 'Ada, L.', note = NULL WHERE id = 1;\n"),
        "{text}"
    );
    let o = ExportOptions {
        table: Some("t".into()),
        ..options(Format::SqlUpdates)
    };
    let text = out(&o);
    assert!(
        text.starts_with(
            "UPDATE t SET id = 1, name = 'Ada, L.', note = NULL WHERE id = 1 AND name = 'Ada, L.' AND note IS NULL;\n"
        ),
        "{text}"
    );
}

#[test]
fn where_clause_matches_the_rows() {
    let rows = vec![
        vec![Value::Int(1), Value::Text("a'b".into()), Value::Null],
        vec![Value::Int(2), Value::Null, Value::Int(3)],
    ];
    assert_eq!(
        to_string(&options(Format::WhereClause), &columns(), &rows),
        "(id = 1 AND name = 'a''b' AND note IS NULL)\nOR (id = 2 AND name IS NULL AND note = 3)"
    );
    // One column: no brackets.
    assert_eq!(
        to_string(
            &options(Format::WhereClause),
            &columns()[..1],
            &[vec![Value::Int(7)], vec![Value::Int(8)]]
        ),
        "id = 7\nOR id = 8"
    );
}

#[test]
fn names_are_quoted_as_the_dialect_needs() {
    let cols = vec![col("Order", ValueKind::Int), col("plain", ValueKind::Int)];
    let o = options(Format::SqlInserts);
    assert_eq!(
        to_string(&o, &cols, &[vec![Value::Int(1), Value::Int(2)]]),
        "INSERT INTO table_name (\"Order\", plain) VALUES (1, 2);\n"
    );
}
