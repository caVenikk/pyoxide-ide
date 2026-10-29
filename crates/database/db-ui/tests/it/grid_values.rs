//! What the grid shows for values, over a scripted connection source: NULL
//! and the empty text apart, JSON, binary display types, a value the page
//! cut and its whole read by the row's key, the value panel and its tabs,
//! the transposed and text views, the aggregates of a selection. The
//! bindings of `keymaps/*/values.toml` are pressed in it as well.

use db_core::{Backend, Purpose, Value as DbValue};
use db_session::fake::{FakeSource, Reaction};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::keymap::Binding;
use workbench::test_support::keymap::{Harness, check_area};

use super::grid::{grid_state, open_customers, store_with_model, wait_grid};
use super::*;

const LONG: usize = 5000;

fn long_text() -> String {
    "abcdefghij".repeat(LONG / 10)
}

/// A server with `demo.customers`: three rows of id, email, profile (JSON),
/// notes (empty, cut, two lines), key (binary), amount, born (a date) and
/// big (numbers to group).
fn server() -> FakeSource {
    let source = FakeSource::new(Backend::Postgres);
    let columns: Vec<String> = [
        "id", "email", "profile", "notes", "key", "amount", "born", "big",
    ]
    .iter()
    .map(|c| (*c).to_owned())
    .collect();
    // The whole notes of row 2, asked for by the key (earlier rules win).
    source.on(
        "where t.id = 2",
        Reaction::Rows {
            columns: vec!["notes".into()],
            rows: vec![vec![DbValue::Text(long_text())]],
        },
    );
    let uuid: Vec<u8> = (0..16).collect();
    source.on(
        "from demo.customers t",
        Reaction::Rows {
            columns,
            rows: vec![
                vec![
                    DbValue::Int(1),
                    DbValue::from("a@example.test"),
                    DbValue::Json(r#"{"a":1,"b":[1,2]}"#.into()),
                    DbValue::Text(String::new()),
                    DbValue::Bytes(uuid),
                    DbValue::Int(10),
                    DbValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 3, 9).unwrap()),
                    DbValue::Decimal("-1234567.891".into()),
                ],
                vec![
                    DbValue::Int(2),
                    DbValue::from("b@example.test"),
                    DbValue::Null,
                    DbValue::Truncated {
                        prefix: Box::new(DbValue::Text(long_text()[..1000].to_owned())),
                        full_len: Some(LONG as u64),
                    },
                    DbValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef]),
                    DbValue::Decimal("2.5".into()),
                    DbValue::Null,
                    DbValue::Int(0),
                ],
                vec![
                    DbValue::Int(3),
                    DbValue::from("c@example.test"),
                    DbValue::Json("{}".into()),
                    DbValue::Text("line one\nline two".into()),
                    DbValue::Null,
                    DbValue::Int(30),
                    DbValue::Null,
                    DbValue::Int(999),
                ],
            ],
        },
    );
    source
}

fn cell_of(s: &Value, row: usize, column: usize) -> Value {
    s["cells"][row][column].clone()
}

#[gpui::test]
fn values_are_typed_and_the_panel_shows_the_whole_one(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    let s = open_customers(&mut h);

    // NULL is no text; the empty text is one.
    assert_eq!(cell_of(&s, 0, 3), json!(""), "{s}");
    assert_eq!(cell_of(&s, 0, 2), json!(r#"{"a":1,"b":[1,2]}"#));
    assert_eq!(cell_of(&s, 1, 2), Value::Null);
    // A UUID-sized binary value reads as a UUID, any other as hex.
    assert_eq!(
        cell_of(&s, 0, 4),
        json!("00010203-0405-0607-0809-0a0b0c0d0e0f")
    );
    assert_eq!(cell_of(&s, 1, 4), json!("0xDEADBEEF"));
    assert_eq!(cell_of(&s, 2, 4), Value::Null);

    // Display types change the cells without reading the page again.
    let statements = server.statements(Purpose::Data).len();
    h.invoke(
        "db.grid.set-display-type",
        json!({"column": 4, "display": "hex"}),
    );
    let s = wait_grid(&mut h, "hex cells", |s| {
        s["cells"][0][4] == "0x000102030405060708090A0B0C0D0E0F"
    });
    assert_eq!(s["values"]["display"]["key"], "hex");
    assert_eq!(server.statements(Purpose::Data).len(), statements);
    h.invoke(
        "db.grid.set-display-type",
        json!({"column": 4, "display": "detect"}),
    );
    wait_grid(&mut h, "detect again", |s| {
        s["cells"][0][4] == "00010203-0405-0607-0809-0a0b0c0d0e0f"
    });

    // The panel opens on the Value tab with the JSON indented.
    h.invoke("db.grid.select", json!({"row": 0, "column": 2}));
    h.invoke("db.grid.value-editor", Value::Null);
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["panel"]["tab"], "value", "{s}");
    assert_eq!(s["values"]["panel"]["is_json"], true);
    let text = s["values"]["panel"]["text"].as_str().unwrap();
    assert!(text.contains("\n  \"a\": 1"), "{text:?}");
    h.invoke("db.grid.value-format-json", Value::Null);
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["panel"]["text"], r#"{"a":1,"b":[1,2]}"#);

    // NULL shows nothing in the panel; the value is marked as NULL.
    h.invoke("db.grid.select", json!({"row": 1, "column": 2}));
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["value"]["null"], true, "{s}");
    assert_eq!(s["values"]["panel"]["text"], "");

    // A cut value says so, then Load Full Value reads it by the key.
    h.invoke("db.grid.select", json!({"row": 1, "column": 3}));
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["value"]["truncated"], true, "{s}");
    assert_eq!(s["values"]["value"]["full_len"], LONG);
    assert_eq!(
        s["values"]["panel"]["text"].as_str().unwrap().len(),
        1000,
        "the prefix"
    );
    h.invoke("db.grid.load-full-value", Value::Null);
    wait_grid(&mut h, "the whole value", |s| {
        s["values"]["value"]["full"]["loaded"].is_number()
    });
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(
        s["values"]["panel"]["text"].as_str().unwrap().len(),
        LONG,
        "{}",
        s["values"]["panel"]
    );
    assert!(
        server
            .statements(Purpose::Data)
            .contains(&"SELECT t.notes\nFROM demo.customers t\nWHERE t.id = 2".to_owned()),
        "{:?}",
        server.statements(Purpose::Data)
    );

    // Save to File writes the whole value.
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("notes.txt");
    h.invoke(
        "db.grid.save-value",
        json!({"path": path.display().to_string()}),
    );
    let started = std::time::Instant::now();
    while !path.exists() || std::fs::metadata(&path).unwrap().len() < LONG as u64 {
        assert!(started.elapsed().as_secs() < 10, "the file");
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.draw();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), long_text());

    // The panel closes.
    h.invoke("db.grid.hide-panel", Value::Null);
    assert!(grid_state(&mut h)["values"]["panel"].is_null());
}

#[gpui::test]
fn the_record_and_aggregates_tabs_follow_the_selection(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);

    h.invoke(
        "db.grid.select",
        json!({"row": 0, "column": 5, "to_row": 2, "to_column": 5}),
    );
    h.invoke("db.grid.aggregate-view", Value::Null);
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["panel"]["tab"], "aggregates", "{s}");
    let a = &s["values"]["aggregates"];
    assert_eq!(a["COUNT"], "3", "{a}");
    assert_eq!(a["COUNT_NUMS"], "3");
    assert_eq!(a["SUM"], "42.5");
    assert_eq!(a["MIN"], "2.5");
    assert_eq!(a["MAX"], "30");
    assert_eq!(a["MEDIAN"], "10");
    assert_eq!(a["ROWS"], "3");
    assert_eq!(a["COLS"], "1");

    // One cell of text: no numbers, so the figures need more values.
    h.invoke("db.grid.select", json!({"row": 0, "column": 1}));
    let s = grid_state(&mut h);
    let a = &s["values"]["aggregates"];
    assert_eq!(a["COUNT"], "1");
    assert_eq!(a["COUNT_NUMS"], "0");
    assert_eq!(a["SUM"], "0");
    assert_eq!(a["AVG"], "Not enough values");

    h.invoke("db.grid.record-view", Value::Null);
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["panel"]["tab"], "record", "{s}");
}

#[gpui::test]
fn the_transposed_and_text_views_show_the_same_page(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);
    let statements = server.statements(Purpose::Data).len();

    h.invoke("db.grid.select", json!({"row": 1, "column": 1}));
    h.invoke("db.grid.set-view", json!({"view": "transpose"}));
    h.draw();
    let s = grid_state(&mut h);
    let t = &s["values"]["transposed"];
    assert_eq!(t["columns"], json!(["", "1", "2", "3"]), "{s}");
    assert_eq!(t["rows"], 8);
    assert_eq!(t["cells"][1][0], "email");
    assert_eq!(t["cells"][1][2], "b@example.test");
    // The current cell stays the same cell of the table: row 2, email.
    assert_eq!(s["selection"]["lead"], json!([1, 2]), "{s}");
    assert_eq!(s["values"]["value"]["row"], 1);
    assert_eq!(s["values"]["value"]["column"], 1);
    assert_eq!(server.statements(Purpose::Data).len(), statements);

    h.invoke("db.grid.set-view", json!({"view": "text"}));
    h.draw();
    let s = grid_state(&mut h);
    let text = s["values"]["text"].as_str().unwrap();
    let first = text.lines().next().unwrap();
    assert!(
        first.starts_with("1,a@example.test,\"{\"\"a\"\":1"),
        "{first}"
    );
    assert!(text.contains("\"line one\nline two\""), "{text:?}");
    // NULL is nothing between the commas.
    assert!(text.contains(",,"), "{text:?}");

    h.invoke("db.grid.set-view", json!({"view": "table"}));
    h.draw();
    let s = grid_state(&mut h);
    assert_eq!(s["values"]["view"], "table");
    assert!(s["values"]["text"].is_null());
    assert_eq!(s["values"]["transposed"], Value::Null);
}

#[gpui::test]
fn the_data_views_page_changes_how_dates_and_numbers_read(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    let s = open_customers(&mut h);
    assert_eq!(cell_of(&s, 0, 6), json!("2024-03-09"), "{s}");
    assert_eq!(cell_of(&s, 0, 7), json!("-1234567.891"));
    let statements = server.statements(Purpose::Data).len();

    h.invoke("settings.open", json!({"page": "database.data-views"}));
    let page = h.invoke("settings.state", Value::Null);
    assert_eq!(page["page"], "database.data-views", "{page}");
    assert_eq!(page["fields"]["date"], "yyyy-MM-dd");
    assert_eq!(page["fields"]["group-numbers"], false);
    let edit = |h: &mut Harness, field: &str, value: Value| {
        h.invoke("settings.edit", json!({"field": field, "value": value}));
    };
    edit(&mut h, "date", json!("dd.MM.yyyy"));
    edit(&mut h, "group-numbers", json!(true));
    edit(&mut h, "decimal-separator", json!(","));
    let s = h.invoke("settings.state", Value::Null);
    assert_eq!(s["fields"]["example"]["number"], "-1 234 567,891", "{s}");
    assert_eq!(s["modified"], json!(["database.data-views"]), "{s}");
    // The grid waits for Apply.
    assert_eq!(cell_of(&grid_state(&mut h), 0, 6), json!("2024-03-09"));

    // A pattern with an open quote keeps the page.
    edit(&mut h, "date", json!("dd'MM"));
    h.invoke("settings.apply", Value::Null);
    let s = h.invoke("settings.state", Value::Null);
    assert!(s["errors"]["database.data-views"].is_string(), "{s}");
    edit(&mut h, "date", json!("dd.MM.yyyy"));
    h.invoke("settings.apply", Value::Null);
    let s = wait_grid(&mut h, "dates in the new pattern", |s| {
        s["cells"][0][6] == "09.03.2024"
    });
    assert_eq!(cell_of(&s, 0, 7), json!("-1 234 567,891"), "{s}");
    assert_eq!(cell_of(&s, 2, 7), json!("999"));
    // Formats change what is shown, not the values or the statements.
    assert_eq!(server.statements(Purpose::Data).len(), statements);
}

// ---- key bindings --------------------------------------------------------------

fn check(h: &mut Harness, b: &Binding) -> bool {
    let command = b.command.as_str();
    h.invoke("db.grid.focus-field", json!({"field": "cells"}));
    h.invoke(
        "db.grid.select",
        json!({"row": 0, "column": 5, "to_row": 2, "to_column": 5}),
    );
    h.press(&b.keys);
    h.draw();
    let s = grid_state(h);
    match command {
        "db.grid.value-editor" => assert_eq!(s["values"]["panel"]["tab"], "value", "{s}"),
        "db.grid.record-view" => assert_eq!(s["values"]["panel"]["tab"], "record", "{s}"),
        "db.grid.copy-aggregation" => {
            let text =
                h.cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
            assert_eq!(text.as_deref(), Some("42.5"), "{s}");
        }
        _ => return false,
    }
    true
}

fn run(cx: &mut TestAppContext, macos: bool) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let area = if macos {
        include_str!("../../keymaps/macos/values.toml")
    } else {
        include_str!("../../keymaps/default/values.toml")
    };
    check_area(cx, product(store), macos, area, Some(&root), |h, b| {
        if grid_state(h).is_null() {
            open_customers(h);
        }
        check(h, b)
    });
}

#[gpui::test]
fn every_macos_value_binding_works(cx: &mut TestAppContext) {
    run(cx, true);
}

#[gpui::test]
fn every_windows_and_linux_value_binding_works(cx: &mut TestAppContext) {
    run(cx, false);
}
