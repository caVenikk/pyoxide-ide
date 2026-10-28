//! The SQL language services in the editor: a file attached to a data source
//! resolves its names through the model (cached on disk or loaded by the
//! Database tool window), gets completion and diagnostics from it, and a
//! file attached to nothing gets syntax errors only.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ::sql_intel::ScopeProvider as _;
use db_core::DataSourceId;
use db_model::{Model, ModelCache};
use db_ui::fixture::demo_snapshot;
use db_ui::store::fake::FakeStore;
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

pub const SOURCE: &str = "local";

/// A store whose cache holds the demo model of `SOURCE`.
pub fn cached_store() -> (Arc<FakeStore>, tempfile::TempDir) {
    let cache_dir = tempfile::tempdir().unwrap();
    ModelCache::new(cache_dir.path())
        .store(&Model::from_snapshot(
            DataSourceId::from(SOURCE),
            demo_snapshot(),
        ))
        .unwrap();
    let store = FakeStore::new("secret");
    store.set_model_cache(cache_dir.path());
    (store, cache_dir)
}

pub fn open<'a>(
    cx: &'a mut TestAppContext,
    store: Arc<FakeStore>,
    root: &Path,
    file: &str,
    text: &str,
) -> Harness<'a> {
    std::fs::write(root.join(file), text).unwrap();
    store.add(root, SOURCE, pg());
    let mut h = Harness::new(cx, product(store), cfg!(target_os = "macos"), Some(root));
    // The services answer from a thread of their own, also while the
    // application quits and waits for it to end.
    h.cx.executor().allow_parking();
    h.invoke(
        "project.open-file",
        json!({ "path": root.join(file).display().to_string() }),
    );
    h
}

fn code_state(h: &mut Harness) -> Value {
    h.invoke("code.state", Value::Null)
}

/// Draws until `code.state` satisfies `f` (the services answer from threads
/// of their own).
fn wait_for(h: &mut Harness, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let s = code_state(h);
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

pub fn attach(h: &mut Harness) {
    h.invoke("sql.attach-data-source", json!({ "name": SOURCE }));
    let started = Instant::now();
    while h.invoke("sql.state", Value::Null)["data-source"] != SOURCE {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the attachment"
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

fn labels(state: &Value) -> Vec<String> {
    state["completion"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i["label"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[gpui::test]
fn a_sql_document_gets_the_sql_services(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "select 1;\n");
    let s = wait_for(&mut h, "the document", |s| {
        s["documents"]
            .as_array()
            .is_some_and(|d| d.iter().any(|d| d["language"] == "sql"))
    });
    assert_eq!(s["services"], true);
}

#[gpui::test]
fn completion_lists_the_tables_of_the_attached_data_source(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "");
    attach(&mut h);

    h.invoke("editor.type", json!({ "text": "select * from demo." }));
    let mut tries = 0;
    let s = loop {
        h.invoke("code.complete", Value::Null);
        let s = wait_for(&mut h, "completion", |s| !s["completion"].is_null());
        // The cached model is read on a thread of its own: the first
        // request may come before it.
        if labels(&s).iter().any(|l| l == "customers") || tries > 200 {
            break s;
        }
        tries += 1;
        h.invoke("completion.cancel", Value::Null);
        std::thread::sleep(Duration::from_millis(20));
    };
    let got = labels(&s);
    for want in ["customers", "orders", "audit_log", "v_order_totals"] {
        assert!(got.iter().any(|l| l == want), "{want} in {got:?}");
    }
}

#[gpui::test]
fn an_unknown_table_is_a_problem_only_once_the_file_is_attached(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (store, _cache) = cached_store();
    let mut h = open(cx, store, &root, "q.sql", "select * from demo.nosuch;\n");
    // Attached to nothing: no metadata, no unresolved reference.
    wait_for(&mut h, "the document", |s| s["problems"] == 0);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(code_state(&mut h)["problems"], 0);

    attach(&mut h);
    wait_for(&mut h, "the unresolved table", |s| {
        s["problems"].as_u64().is_some_and(|p| p >= 1)
    });
}

#[test]
fn the_hub_scope_follows_the_file_binding_and_the_published_model() {
    let (_dir, root) = project();
    let store = FakeStore::new("secret");
    store.add(&root, SOURCE, pg());
    let hub = Arc::new(db_ui::sql::Hub::new(store));
    let project = hub.read_project(&root);
    hub.install(&root, project);
    hub.update(&root, db_ui::sql::Kind::Personal, |s| {
        s.set_scope(None, Some((SOURCE, Some("demo"))));
    })
    .unwrap();

    let scope = db_ui::sql::HubScope::new(hub.clone(), &root);
    let file = code_intel::DocumentId::new(root.join("a.sql"));
    let ctx = scope.context(&file);
    assert_eq!(ctx.dialect, sql_syntax::Dialect::Postgres);
    // Nothing cached and nothing published: names stay unknown.
    assert!(ctx.scope.is_none());

    let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = notified.clone();
    hub.subscribe(move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    });
    let model = Arc::new(Model::from_snapshot(
        DataSourceId::from(SOURCE),
        demo_snapshot(),
    ));
    hub.publish_model(&root, SOURCE, model.clone());
    assert_eq!(notified.load(std::sync::atomic::Ordering::SeqCst), 1);
    let scope_of = scope.context(&file).scope.expect("a scope");
    assert!(Arc::ptr_eq(&scope_of.model, &model));
    assert_eq!(scope_of.search_path, ["demo"]);

    // A console lives outside the project and is bound to its data source
    // by where it is: its scope is the data source's model, in its schema.
    let console = code_intel::DocumentId::new("/config/consoles/local/console.sql");
    hub.bind_console(
        console.path(),
        db_ui::sql::ConsoleBinding {
            source: db_ui::sql::Source {
                name: SOURCE.into(),
                dialect: sql_syntax::Dialect::Postgres,
                database: None,
            },
            schema: Some("public".into()),
        },
    );
    let ctx = scope.context(&console);
    assert_eq!(ctx.dialect, sql_syntax::Dialect::Postgres);
    let scope_of = ctx.scope.expect("a console scope");
    assert!(Arc::ptr_eq(&scope_of.model, &model));
    assert_eq!(scope_of.search_path, ["public"]);
    let seen_by_console = notified.load(std::sync::atomic::Ordering::SeqCst);

    // The same model again changes nothing.
    hub.publish_model(&root, SOURCE, model);
    assert_eq!(
        notified.load(std::sync::atomic::Ordering::SeqCst),
        seen_by_console
    );
}
