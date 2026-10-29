//! The metadata of a data source for a SQL file's completion, when the
//! Database tool window has not loaded it: a data source that needs no
//! prompt is loaded in the background, once; one that does shows a hint
//! with a "Connect" link in the file's toolbar and connects nothing until
//! the link is used. Fake stores only: no credential store is read.

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{Backend, DataSourceId, Profile, Secret};
use db_model::Model;
use db_ui::fixture::demo_snapshot;
use db_ui::store::DataSourceStore as _;
use db_ui::store::fake::{FakeStore, NoServer};
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

const SOURCE: &str = "local";

/// A project with `q.sql` open in a window over a store that has the data
/// source `SOURCE` of `profile` and no cached model; the server refuses
/// every connection and counts them.
fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
    profile: Profile,
    kept: Option<&str>,
) -> (Harness<'a>, Arc<NoServer>, Arc<FakeStore>) {
    let store = FakeStore::new("secret");
    let server = Arc::new(NoServer::default());
    store.set_connections(server.clone());
    store.add(root, SOURCE, profile);
    if let Some(password) = kept {
        store.hold_password(root, SOURCE, Secret::new(password));
    }
    std::fs::write(root.join("q.sql"), "select 1;\n").unwrap();
    let mut h = Harness::new(
        cx,
        product(store.clone()),
        cfg!(target_os = "macos"),
        Some(root),
    );
    // The services and the data source handles answer from threads of
    // their own.
    h.cx.executor().allow_parking();
    h.invoke(
        "project.open-file",
        json!({ "path": root.join("q.sql").display().to_string() }),
    );
    (h, server, store)
}

/// Draws until `f` holds.
fn wait(h: &mut Harness, what: &str, f: impl Fn(&mut Harness) -> bool) {
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

fn sql(h: &mut Harness) -> Value {
    h.invoke("sql.state", Value::Null)
}

fn attach(h: &mut Harness) {
    h.invoke("sql.attach-data-source", json!({ "name": SOURCE }));
    wait(h, "the attachment", |h| sql(h)["data-source"] == SOURCE);
}

/// The titles of the notifications so far.
fn notified(h: &mut Harness) -> Vec<String> {
    h.invoke("workbench.state", Value::Null)["notifications"]["timeline"]
        .as_array()
        .map(|all| {
            all.iter()
                .map(|n| format!("{}: {}", n["title"], n["content"]))
                .collect()
        })
        .unwrap_or_default()
}

fn file_source() -> Profile {
    let mut p = Profile::new(Backend::Sqlite);
    p.path = Some("/tmp/none.db".into());
    p
}

/// A password is kept: attaching the file connects without a word, tells
/// the failure of a server that is not there once, and never retries.
#[gpui::test]
fn a_kept_password_loads_the_model_in_the_background(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, server, _store) = open(cx, &root, pg(), Some("secret"));
    attach(&mut h);

    wait(&mut h, "the connection", |_| server.opened() >= 1);
    wait(&mut h, "the failure", |h| {
        notified(h)
            .iter()
            .any(|n| n.contains(NoServer::MESSAGE) && n.contains(SOURCE))
    });
    assert!(sql(&mut h)["connect-hint"].is_null());
    assert!(state(&mut h)["prompt"].is_null());

    // No retry loop: nothing more is asked of the server.
    let opened = server.opened();
    std::thread::sleep(Duration::from_millis(300));
    h.draw();
    assert_eq!(server.opened(), opened);
    assert_eq!(
        notified(&mut h)
            .iter()
            .filter(|n| n.contains(NoServer::MESSAGE))
            .count(),
        1
    );
}

/// A data source with no password at all (a file) is loaded the same way.
#[gpui::test]
fn a_data_source_without_a_password_loads_in_the_background(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, server, _store) = open(cx, &root, file_source(), None);
    attach(&mut h);
    wait(&mut h, "the connection", |_| server.opened() >= 1);
    assert!(sql(&mut h)["connect-hint"].is_null());
    assert!(state(&mut h)["prompt"].is_null());
}

/// No password to be had without asking: nothing connects, nothing is
/// asked, the toolbar offers to connect; the link asks for the password,
/// holds it for the run and connects with it.
#[gpui::test]
fn a_data_source_that_needs_a_prompt_shows_the_hint_and_waits(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, server, store) = open(cx, &root, pg(), None);
    // Nothing attached: no hint.
    assert!(sql(&mut h)["connect-hint"].is_null());
    attach(&mut h);

    wait(&mut h, "the hint", |h| sql(h)["connect-hint"] == SOURCE);
    std::thread::sleep(Duration::from_millis(300));
    h.draw();
    assert_eq!(server.opened(), 0, "nothing connects by itself");
    assert!(state(&mut h)["prompt"].is_null(), "nothing is asked");
    assert!(notified(&mut h).is_empty());

    h.invoke("sql.connect", Value::Null);
    wait(&mut h, "the prompt", |h| !state(h)["prompt"].is_null());
    assert_eq!(state(&mut h)["prompt"]["name"], SOURCE);
    h.invoke("db.password-prompt.set", json!({"value": "secret"}));
    h.invoke("db.password-prompt.ok", Value::Null);
    wait(&mut h, "the connection", |_| server.opened() >= 1);
    assert_eq!(store.password(SOURCE).as_deref(), Some("secret"));
    // The fake server is not there: no model, so the hint stays for another
    // try (which needs no password now).
    wait(&mut h, "the failure", |h| {
        notified(h).iter().any(|n| n.contains(NoServer::MESSAGE))
    });
    assert_eq!(sql(&mut h)["connect-hint"], SOURCE);
    // The refresh that failed may still be settling: let it end first.
    std::thread::sleep(Duration::from_millis(200));
    h.draw();
    let opened = server.opened();
    h.invoke("sql.connect", Value::Null);
    wait(&mut h, "the second try", |_| server.opened() > opened);
    assert!(state(&mut h)["prompt"].is_null());
}

/// Cancelling the prompt connects nothing and keeps the hint.
#[gpui::test]
fn cancelling_the_prompt_keeps_the_hint(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, server, store) = open(cx, &root, pg(), None);
    attach(&mut h);
    wait(&mut h, "the hint", |h| sql(h)["connect-hint"] == SOURCE);

    h.invoke("sql.connect", Value::Null);
    wait(&mut h, "the prompt", |h| !state(h)["prompt"].is_null());
    h.invoke("db.password-prompt.cancel", Value::Null);
    std::thread::sleep(Duration::from_millis(200));
    h.draw();
    assert!(state(&mut h)["prompt"].is_null());
    assert_eq!(server.opened(), 0);
    assert_eq!(store.password(SOURCE), None);
    assert_eq!(sql(&mut h)["connect-hint"], SOURCE);
}

/// The hint goes when the model arrives, from whoever loads it.
#[test]
fn the_hint_goes_when_a_model_is_published() {
    let (_dir, root) = project();
    let store = FakeStore::new("secret");
    store.add(&root, SOURCE, pg());
    let hub = db_ui::sql::Hub::new(store);
    assert!(!hub.connect_hint(&root, SOURCE));

    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = heard.clone();
    hub.subscribe(move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    });
    hub.set_needs_connect(&root, SOURCE, true);
    assert!(hub.connect_hint(&root, SOURCE));
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 1);
    // Said again: nothing changes.
    hub.set_needs_connect(&root, SOURCE, true);
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 1);

    hub.publish_model(
        &root,
        SOURCE,
        Arc::new(Model::from_snapshot(
            DataSourceId::from(SOURCE),
            demo_snapshot(),
        )),
    );
    assert!(!hub.connect_hint(&root, SOURCE));
}
