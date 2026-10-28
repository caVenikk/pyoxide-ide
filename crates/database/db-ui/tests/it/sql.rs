//! The SQL editor glue: a `.sql` file is coloured by the SQL highlighter,
//! the status bar widget shows its dialect and scope, and a choice in it
//! is a mapping that is kept, reaches the open editor and survives a
//! restart.

use std::time::{Duration, Instant};

use db_ui::store::fake::FakeStore;
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

fn open_sql<'a>(cx: &'a mut TestAppContext, root: &Path, file: &str) -> Harness<'a> {
    let store = FakeStore::new("secret");
    store.add(root, "local", pg());
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(root));
    h.invoke(
        "project.open-file",
        json!({ "path": root.join(file).display().to_string() }),
    );
    h
}

fn sql_state(h: &mut Harness) -> Value {
    h.invoke("sql.state", Value::Null)
}

/// Draws until the widget's state satisfies `f`.
fn wait_for(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = sql_state(h);
        if f(&s) {
            return s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {s}"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

fn roles(h: &mut Harness, line: usize) -> Vec<(String, String)> {
    let v = h.invoke("editor.highlights", json!({ "line": line }));
    let text = h.invoke("editor.text", Value::Null);
    let text = text.as_str().unwrap().lines().nth(line).unwrap().to_owned();
    v["spans"]
        .as_array()
        .unwrap_or_else(|| panic!("spans in {v}"))
        .iter()
        .map(|s| {
            let (start, end) = (s["start"].as_u64().unwrap(), s["end"].as_u64().unwrap());
            let piece: String = text
                .chars()
                .skip(start as usize)
                .take((end - start) as usize)
                .collect();
            (piece, s["role"].as_str().unwrap_or_default().to_owned())
        })
        .collect()
}

fn project_with_files() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = project();
    std::fs::create_dir_all(root.join("migrations")).unwrap();
    std::fs::write(
        root.join("scratch.sql"),
        "select name, 'a' from customers c -- note\nwhere c.id = 1;\n",
    )
    .unwrap();
    std::fs::write(root.join("migrations/001.sql"), "select 1;\n").unwrap();
    (dir, root)
}

#[gpui::test]
fn a_sql_file_is_coloured_and_the_widget_shows_generic_sql(cx: &mut TestAppContext) {
    let (_dir, root) = project_with_files();
    let mut h = open_sql(cx, &root, "scratch.sql");
    let s = wait_for(&mut h, "the widget", |s| s["visible"] == true);
    assert_eq!(s["text"], "Generic SQL", "{s}");
    assert_eq!(s["dialect-origin"]["kind"], "none");
    assert!(s["data-source"].is_null(), "{s}");
    assert_eq!(s["sources"], json!(["local"]));

    // The lexical layer colours the line at once; the tree gives the names
    // their roles after the background pass.
    let first = roles(&mut h, 0);
    assert!(
        first.contains(&("select".into(), "keyword".into())),
        "{first:?}"
    );
    assert!(
        first.contains(&("'a'".into(), "string".into())),
        "{first:?}"
    );
    assert!(
        first.contains(&("-- note".into(), "comment".into())),
        "{first:?}"
    );
    let started = Instant::now();
    loop {
        let spans = roles(&mut h, 0);
        if spans.contains(&("customers".into(), "sql-table".into())) {
            assert!(
                spans.contains(&("name".into(), "sql-column".into())),
                "{spans:?}"
            );
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{spans:?}");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

#[gpui::test]
fn the_widget_is_hidden_for_other_files(cx: &mut TestAppContext) {
    let (_dir, root) = project_with_files();
    std::fs::write(root.join("notes.txt"), "x\n").unwrap();
    let mut h = open_sql(cx, &root, "notes.txt");
    h.draw();
    assert_eq!(sql_state(&mut h)["visible"], false);
}

#[gpui::test]
fn a_folder_mapping_changes_the_dialect_and_is_kept(cx: &mut TestAppContext) {
    let (_dir, root) = project_with_files();
    let mut h = open_sql(cx, &root, "migrations/001.sql");
    wait_for(&mut h, "the widget", |s| s["visible"] == true);

    h.invoke(
        "sql.set-dialect",
        json!({ "dialect": "postgres", "target": "folder" }),
    );
    let s = wait_for(&mut h, "the mapping", |s| s["dialect"] == "postgres");
    assert_eq!(s["text"], "PostgreSQL");
    assert_eq!(
        s["dialect-origin"],
        json!({"kind": "mapping", "path": "migrations", "personal": true})
    );

    // Personal: in the workspace file, never in the shared settings.
    let workspace = std::fs::read_to_string(root.join(".oxide/workspace.toml")).unwrap();
    assert!(workspace.contains("[[sql.mappings]]"), "{workspace}");
    assert!(workspace.contains("path = \"migrations\""), "{workspace}");
    assert!(!root.join(".oxide/project.toml").exists());

    // A restart reads the same mapping.
    let hub = db_ui::sql::Hub::new(FakeStore::new("secret"));
    let project = hub.read_project(&root);
    hub.install(&root, project);
    assert_eq!(
        hub.dialect(&root.join("migrations/002.sql")),
        sql_syntax::Dialect::Postgres
    );
    assert_eq!(
        hub.dialect(&root.join("scratch.sql")),
        sql_syntax::Dialect::Generic
    );
}

#[gpui::test]
fn attaching_a_data_source_gives_the_dialect_and_a_shared_mapping_goes_to_the_project(
    cx: &mut TestAppContext,
) {
    let (_dir, root) = project_with_files();
    let mut h = open_sql(cx, &root, "scratch.sql");
    wait_for(&mut h, "the widget", |s| s["visible"] == true);

    h.invoke(
        "sql.attach-data-source",
        json!({ "name": "local", "schema": "public" }),
    );
    let s = wait_for(&mut h, "the attachment", |s| s["data-source"] == "local");
    assert_eq!(s["schema"], "public");
    assert_eq!(s["dialect"], "postgres");
    assert_eq!(
        s["dialect-origin"],
        json!({"kind": "data-source", "name": "local"})
    );
    assert_eq!(s["data-source-text"], "local.public");

    h.invoke("sql.attach-data-source", json!({}));
    let s = wait_for(&mut h, "the detach", |s| s["data-source"].is_null());
    assert_eq!(s["dialect"], "generic");

    h.invoke(
        "sql.set-dialect",
        json!({ "dialect": "oracle", "target": "project", "shared": true }),
    );
    let s = wait_for(&mut h, "the project default", |s| s["dialect"] == "oracle");
    assert_eq!(s["dialect-origin"], json!({"kind": "project"}));
    let shared = std::fs::read_to_string(root.join(".oxide/project.toml")).unwrap();
    assert!(shared.contains("[sql]"), "{shared}");
    assert!(shared.contains("dialect = \"oracle\""), "{shared}");
}

#[gpui::test]
fn the_popup_lists_the_dialects_and_the_data_sources(cx: &mut TestAppContext) {
    let (_dir, root) = project_with_files();
    let mut h = open_sql(cx, &root, "migrations/001.sql");
    wait_for(&mut h, "the widget", |s| s["visible"] == true);
    h.invoke("sql.choose", Value::Null);
    let s = wait_for(&mut h, "the popup", |s| !s["popup"].is_null());
    let text = s["popup"].to_string();
    for label in [
        "Generic SQL",
        "PostgreSQL",
        "Oracle",
        "SQLite",
        "Use for the Folder",
        "Use for the Project",
        "Attach Data Source",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
}

#[gpui::test]
fn a_new_dialect_colours_the_open_editor_again(cx: &mut TestAppContext) {
    let (_dir, root) = project_with_files();
    std::fs::write(root.join("q.sql"), "select q'[it's]' from dual\n").unwrap();
    let mut h = open_sql(cx, &root, "q.sql");
    wait_for(&mut h, "the widget", |s| s["visible"] == true);
    let generic = roles(&mut h, 0);
    assert!(
        !generic.contains(&("q'[it's]'".into(), "string".into())),
        "{generic:?}"
    );

    h.invoke("sql.set-dialect", json!({ "dialect": "oracle" }));
    wait_for(&mut h, "the mapping", |s| s["dialect"] == "oracle");
    let started = Instant::now();
    loop {
        let oracle = roles(&mut h, 0);
        if oracle.contains(&("q'[it's]'".into(), "string".into())) {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{oracle:?}");
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}
