//! Copying from the grid, the Export Data dialog and the Import dialog over
//! a scripted connection source: the preview follows the choices, a file is
//! written in the background and announced by a balloon, the clipboard gets
//! the selection in the toolbar's extractor, and an import maps its columns,
//! runs in one transaction and names the row that failed.

use std::time::{Duration, Instant};

use db_core::Value as DbValue;
use db_session::fake::Reaction;
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::grid::{NAME, grid_state, open_customers, server, store_with_model};
use super::*;

fn clipboard(h: &mut Harness) -> Option<String> {
    h.cx.read_from_clipboard().and_then(|c| c.text())
}

fn transfer_state(h: &mut Harness) -> Value {
    h.invoke("db.transfer.state", Value::Null)
}

/// Draws until `f` holds for the open dialog's state.
fn wait_dialog(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = transfer_state(h);
        if f(&s["state"]) {
            return s["state"].clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {s}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

fn wait_for(h: &mut Harness, what: &str, f: impl Fn(&mut Harness) -> bool) {
    let started = Instant::now();
    while !f(h) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

/// The balloons' texts so far.
fn balloons(h: &mut Harness) -> Vec<String> {
    h.state()["notifications"]["timeline"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|n| n["content"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Chooses a row of the dialog's extractor list: its command runs in the
/// dialog, as a click on the row does.
fn choose_in_export_list(h: &mut Harness, format: &str) {
    use gpui::AppContext as _;
    let main: gpui::AnyWindowHandle = h.window.into();
    let dialog =
        h.cx.update(|cx| ui::dialog_window::modal_of(main, cx))
            .expect("the dialog");
    let action = db_ui::transfer::ExportSetFormat {
        format: format.to_owned(),
    };
    h.cx.update_window(dialog, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    h.draw();
}

fn preview_has(s: &Value, text: &str) -> bool {
    s["preview_loading"] == false && s["preview"].as_str().is_some_and(|p| p.contains(text))
}

#[gpui::test]
fn copy_writes_the_selection_in_the_toolbars_extractor(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);

    h.invoke(
        "db.grid.select",
        json!({"row": 0, "column": 0, "to_row": 1, "to_column": 1}),
    );
    h.invoke("db.grid.copy", Value::Null);
    assert_eq!(
        clipboard(&mut h).as_deref(),
        Some("1,a@example.test\n2,b@example.test\n")
    );

    h.invoke("db.grid.set-extractor", json!({"format": "tsv"}));
    h.invoke("db.grid.copy", Value::Null);
    assert_eq!(
        clipboard(&mut h).as_deref(),
        Some("1\ta@example.test\n2\tb@example.test\n")
    );

    // Another format for one copy; the toolbar's stays.
    h.invoke("db.grid.copy-as", json!({"format": "json"}));
    let text = clipboard(&mut h).unwrap();
    assert!(text.contains("\"email\": \"b@example.test\""), "{text}");
    h.invoke("db.grid.copy-as", json!({"format": "sql-inserts"}));
    let text = clipboard(&mut h).unwrap();
    assert!(
        text.contains("INSERT INTO demo.customers") && text.contains("'b@example.test'"),
        "{text}"
    );
    h.invoke("db.grid.copy-as", json!({"format": "markdown"}));
    let text = clipboard(&mut h).unwrap();
    assert!(
        text.contains("| id") && text.contains("b@example.test"),
        "{text}"
    );

    // The whole table, in the toolbar's extractor, as a background task.
    h.invoke("db.grid.set-extractor", json!({"format": "csv"}));
    h.invoke("db.grid.export-table-to-clipboard", Value::Null);
    wait_for(&mut h, "the table on the clipboard", |h| {
        clipboard(h).is_some_and(|t| t.contains("3,c@example.test"))
    });
    wait_for(&mut h, "the balloon", |h| {
        balloons(h)
            .iter()
            .any(|b| b.contains("copied to the clipboard"))
    });
    let s = grid_state(&mut h);
    assert_eq!(s["selection"]["lead"], json!([1, 1]), "{s}");
}

#[gpui::test]
fn the_export_dialog_previews_and_writes_a_file(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);

    h.invoke("db.export.open", Value::Null);
    let s = wait_dialog(&mut h, "the preview", |s| {
        preview_has(s, "2,b@example.test")
    });
    assert_eq!(s["source"], "demo.customers", "{s}");
    assert_eq!(s["extractor"], "CSV");
    assert!(
        s["path"].as_str().unwrap().ends_with("customers.csv"),
        "{s}"
    );
    assert_eq!(
        s["preview"],
        "1,a@example.test\n2,b@example.test\n3,c@example.test\n"
    );

    // The preview follows the extractor and the options; the suggested file
    // name follows the extractor too.
    h.invoke("db.export.set-format", json!({"format": "tsv"}));
    let s = wait_dialog(&mut h, "tab separated", |s| {
        preview_has(s, "1\ta@example.test")
    });
    assert!(
        s["path"].as_str().unwrap().ends_with("customers.tsv"),
        "{s}"
    );
    h.invoke(
        "db.export.set-option",
        json!({"option": "column-header", "value": true}),
    );
    wait_dialog(&mut h, "a header", |s| preview_has(s, "id\temail\n1\t"));
    h.invoke(
        "db.export.set-option",
        json!({"option": "row-header", "value": true}),
    );
    wait_dialog(&mut h, "row numbers", |s| {
        preview_has(s, "1\t1\ta@example.test")
    });
    h.invoke(
        "db.export.set-option",
        json!({"option": "transpose", "value": true}),
    );
    wait_dialog(&mut h, "transposed", |s| preview_has(s, "id\t"));

    // The extractor list is a popup of its own; a statement extractor
    // writes the table's name.
    h.invoke("db.export.format-menu", Value::Null);
    assert_eq!(transfer_state(&mut h)["state"]["list_open"], true);
    choose_in_export_list(&mut h, "sql-inserts");
    let s = wait_dialog(&mut h, "inserts", |s| {
        preview_has(s, "INSERT INTO demo.customers")
    });
    assert_eq!(s["list_open"], false, "{s}");

    // Back to CSV, written to a file that the dialog's end announces.
    h.invoke("db.export.set-format", json!({"format": "csv"}));
    h.invoke(
        "db.export.set-option",
        json!({"option": "transpose", "value": false}),
    );
    h.invoke(
        "db.export.set-option",
        json!({"option": "row-header", "value": false}),
    );
    h.invoke(
        "db.export.set-option",
        json!({"option": "column-header", "value": false}),
    );
    let out = root.join("customers-out.csv");
    h.invoke(
        "db.export.set-path",
        json!({"path": out.display().to_string()}),
    );
    h.invoke("db.export.to-file", Value::Null);
    assert!(transfer_state(&mut h).is_null(), "the dialog closed");
    wait_for(&mut h, "the file", |_| out.exists());
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "1,a@example.test\n2,b@example.test\n3,c@example.test\n"
    );
    wait_for(&mut h, "the balloon", |h| {
        balloons(h)
            .iter()
            .any(|b| b.contains("3 rows saved from customers to customers-out.csv"))
    });
    // The temporary file is gone.
    let leftovers: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[gpui::test]
fn cancel_closes_the_export_dialog_and_writes_nothing(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);
    h.invoke("db.export.open", Value::Null);
    wait_dialog(&mut h, "the preview", |s| preview_has(s, "a@example.test"));
    let out = root.join("never.csv");
    h.invoke(
        "db.export.set-path",
        json!({"path": out.display().to_string()}),
    );
    h.invoke("db.export.cancel", Value::Null);
    assert!(transfer_state(&mut h).is_null());
    h.draw();
    assert!(!out.exists());
}

/// A server for imports into `demo.customers` (`id`, `email`).
fn import_server() -> db_session::fake::FakeSource {
    let server = server();
    server.on(
        "where 1 = 0",
        Reaction::Rows {
            columns: vec!["id".into(), "email".into()],
            rows: Vec::<Vec<DbValue>>::new(),
        },
    );
    server
}

fn csv_file(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
    let path = dir.join("people.csv");
    std::fs::write(&path, text).unwrap();
    path
}

#[gpui::test]
fn the_import_dialog_maps_columns_and_runs_one_transaction(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = import_server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);
    let file = csv_file(&root, "id,email\n1,x@e.test\n2,\"y, z@e.test\"\n3,\n");

    h.invoke(
        "db.import.open",
        json!({"source": NAME, "schema": "demo", "table": "customers",
               "path": file.display().to_string()}),
    );
    let s = wait_dialog(&mut h, "the file's preview", |s| {
        !s["file_columns"].is_null()
    });
    assert_eq!(s["table"], "demo.customers");
    assert_eq!(s["delimiter"], "comma");
    assert_eq!(s["encoding"], "UTF-8");
    assert_eq!(s["file_columns"], json!(["id", "email"]));
    assert_eq!(s["mapping"][0]["label"], "<Auto> (id)", "{s}");
    assert_eq!(s["mapping"][1]["label"], "<Auto> (email)", "{s}");
    assert_eq!(s["preview_rows"][1], json!(["2", "y, z@e.test"]));
    // An empty field is NULL.
    assert_eq!(s["preview_rows"][2], json!(["3", null]));
    assert_eq!(s["can_import"], true);

    // A wrong delimiter leaves one column and nothing to pair.
    h.invoke("db.import.set-delimiter", json!({"delimiter": "semicolon"}));
    let s = wait_dialog(&mut h, "one column", |s| {
        s["file_columns"] == json!(["id,email"])
    });
    assert_eq!(s["mapping"][1]["label"], "<Not mapped>", "{s}");
    h.invoke("db.import.set-delimiter", json!({"delimiter": "comma"}));
    wait_dialog(&mut h, "two columns", |s| {
        s["file_columns"] == json!(["id", "email"])
    });

    // A choice of the user: the second column takes the file's first.
    h.invoke("db.import.map", json!({"column": "email", "source": 0}));
    let s = transfer_state(&mut h)["state"].clone();
    assert_eq!(s["mapping"][1]["label"], "id", "{s}");
    h.invoke("db.import.map", json!({"column": "email", "source": 1}));
    // Without a header the first record is data.
    h.invoke(
        "db.import.set-option",
        json!({"option": "header", "value": false}),
    );
    let s = wait_dialog(&mut h, "no header", |s| {
        s["file_columns"] == json!(["Column1", "Column2"])
    });
    assert_eq!(s["preview_rows"][0], json!(["id", "email"]));
    h.invoke(
        "db.import.set-option",
        json!({"option": "header", "value": true}),
    );
    wait_dialog(&mut h, "the header", |s| {
        s["file_columns"] == json!(["id", "email"])
    });

    h.invoke("db.import.ok", Value::Null);
    assert!(transfer_state(&mut h).is_null(), "the dialog closed");
    wait_for(&mut h, "the balloon", |h| {
        balloons(h)
            .iter()
            .any(|b| b.contains("people.csv imported to customers: 3 rows"))
    });
    let log: Vec<String> = server.log().into_iter().map(|e| e.sql).collect();
    let insert = log
        .iter()
        .find(|q| q.starts_with("INSERT INTO demo.customers (id, email)"))
        .unwrap_or_else(|| panic!("{log:?}"));
    assert!(insert.matches("($").count() == 3, "{insert}");
    let at = |needle: &str| log.iter().position(|q| q.contains(needle));
    assert!(at("<commit>") > at("INSERT INTO"), "{log:?}");
}

#[gpui::test]
fn a_row_that_fails_is_named_and_the_table_stays(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = import_server();
    server.on(
        "insert into",
        Reaction::query_error("23505", "duplicate key value violates unique constraint"),
    );
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);
    let file = csv_file(&root, "id,email\n1,x@e.test\n2,y@e.test\n");
    h.invoke(
        "db.import.open",
        json!({"source": NAME, "schema": "demo", "table": "customers",
               "path": file.display().to_string()}),
    );
    wait_dialog(&mut h, "the file's preview", |s| {
        !s["file_columns"].is_null()
    });
    h.invoke("db.import.ok", Value::Null);
    wait_for(&mut h, "the error balloon", |h| {
        balloons(h)
            .iter()
            .any(|b| b.contains("Row 1 (line 2)") && b.contains("The table is unchanged"))
    });
    let log: Vec<String> = server.log().into_iter().map(|e| e.sql).collect();
    assert!(log.iter().any(|q| q.contains("<rollback>")), "{log:?}");
    assert!(!log.iter().any(|q| q.contains("<commit>")), "{log:?}");
}

#[gpui::test]
fn the_import_dialog_with_no_mapped_column_cannot_run(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let server = import_server();
    let (store, _cache) = store_with_model(&root, &server);
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(&root));
    open_customers(&mut h);
    let file = csv_file(&root, "a,b\n1,2\n");
    h.invoke(
        "db.import.open",
        json!({"source": NAME, "schema": "demo", "table": "customers",
               "path": file.display().to_string()}),
    );
    let s = wait_dialog(&mut h, "the preview", |s| !s["file_columns"].is_null());
    assert_eq!(s["can_import"], false, "{s}");
    h.invoke("db.import.ok", Value::Null);
    assert_eq!(transfer_state(&mut h)["dialog"], "import", "OK did nothing");
    h.invoke("db.import.cancel", Value::Null);
    assert!(transfer_state(&mut h).is_null());
    assert!(server.log().iter().all(|e| !e.sql.contains("INSERT")));
}
