//! Connecting a data source whose password is not at hand ("Save: Never",
//! or nothing kept yet): every way of connecting opens the password prompt
//! (user, password, how long it is kept) and goes on with what was typed;
//! Disconnect ends the connections, shows it, and a "Never" password is
//! forgotten with it. Over a scripted server that accepts any connection.

use std::sync::Arc;
use std::time::{Duration, Instant};

use db_core::{Backend, GuiOptions, PasswordSave, Purpose};
use db_session::fake::{FakeSource, OpenBehavior, Reaction};
use db_ui::store::DataSourceStore as _;
use db_ui::store::fake::FakeStore;
use gpui::TestAppContext;
use serde_json::{Value, json};
use workbench::test_support::keymap::Harness;

use super::*;

const SOURCE: &str = "local";

fn profile(save: PasswordSave) -> db_core::Profile {
    let mut p = pg();
    p.gui = Some(GuiOptions {
        password_save: save,
        ..GuiOptions::default()
    });
    p
}

/// A project over a store with the data source `SOURCE` that saves its
/// password as `save` and has none; the server takes any connection.
fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
    save: PasswordSave,
) -> (Harness<'a>, Arc<FakeStore>, FakeSource, tempfile::TempDir) {
    let server = FakeSource::new(Backend::Postgres);
    server.on("select 1", Reaction::column("a", &["1"]));
    let store = FakeStore::new("secret");
    store.set_connections(Arc::new(server.clone()));
    store.add(root, SOURCE, profile(save));
    let consoles = tempfile::tempdir().unwrap();
    store.set_console_folder(consoles.path());
    let mut h = Harness::new(
        cx,
        product(store.clone()),
        cfg!(target_os = "macos"),
        Some(root),
    );
    h.cx.executor().allow_parking();
    show_tool_window(&mut h);
    (h, store, server, consoles)
}

fn wait(h: &mut Harness, what: &str, f: impl Fn(&mut Harness) -> bool) {
    let started = Instant::now();
    while !f(h) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waiting for {what}: {}",
            state(h)
        );
        std::thread::sleep(Duration::from_millis(10));
        h.draw();
    }
}

/// Lets what is under way settle.
fn settle(h: &mut Harness) {
    std::thread::sleep(Duration::from_millis(300));
    h.draw();
}

fn asking(h: &mut Harness) -> bool {
    !state(h)["prompt"].is_null()
}

fn source_state(h: &mut Harness) -> Value {
    state(h)["explorer"]["sources"][0].clone()
}

fn connected(h: &mut Harness) -> bool {
    source_state(h)["connected"] == true
}

fn can_disconnect(h: &mut Harness) -> bool {
    state(h)["explorer"]["can_disconnect"] == true
}

fn type_password(h: &mut Harness) {
    h.invoke("db.password-prompt.set", json!({"value": "secret"}));
    h.invoke("db.password-prompt.ok", Value::Null);
}

fn select(h: &mut Harness) {
    h.invoke("db.explorer.select", json!({"name": SOURCE}));
}

fn notified(h: &mut Harness) -> String {
    h.invoke("workbench.state", Value::Null)["notifications"]["timeline"].to_string()
}

fn console(h: &mut Harness) -> Value {
    h.invoke("db.console.state", Value::Null)["consoles"][0].clone()
}

/// Connecting from the tree asks for the password, connects with what was
/// typed and marks the data source connected; Disconnect ends it, the
/// "Never" password goes with it and the next connect asks again. Nothing
/// of the command line's "no password" reaches the person.
#[gpui::test]
fn a_never_data_source_asks_on_every_connect_and_disconnects(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, server, _consoles) = open(cx, &root, PasswordSave::Never);
    select(&mut h);
    assert!(!can_disconnect(&mut h), "nothing is connected");

    for round in 1..=2 {
        h.invoke("db.explorer.refresh", Value::Null);
        wait(&mut h, "the prompt", asking);
        let prompt = state(&mut h)["prompt"].clone();
        assert_eq!(prompt["name"], SOURCE, "round {round}");
        assert_eq!(prompt["user"], "pyoxide", "round {round}");
        assert_eq!(prompt["save"], "never", "the data source's choice shows");
        type_password(&mut h);
        wait(&mut h, "the connection", connected);
        assert!(can_disconnect(&mut h), "round {round}");
        assert_eq!(store.password(SOURCE).as_deref(), Some("secret"));

        // Connected: a refresh asks nothing.
        h.invoke("db.explorer.refresh", Value::Null);
        settle(&mut h);
        assert!(!asking(&mut h), "round {round}");

        h.invoke("db.explorer.disconnect", Value::Null);
        wait(&mut h, "the disconnect", |h| !connected(h));
        assert!(!can_disconnect(&mut h), "round {round}");
        wait(&mut h, "the password forgotten", |_| {
            store.password(SOURCE).is_none()
        });
    }
    assert!(server.opened(Purpose::Meta) >= 2);
    assert!(!notified(&mut h).contains("no password"));
}

/// A cached model shows without the database; Refresh is still a request to
/// connect, the first time as well, and asks for the password.
#[gpui::test]
fn refresh_over_a_cached_model_asks_for_the_password(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, _server, _consoles) = open(cx, &root, PasswordSave::Never);
    let cache_dir = tempfile::tempdir().unwrap();
    db_model::ModelCache::new(cache_dir.path())
        .store(&db_model::Model::from_snapshot(
            db_core::DataSourceId::from(SOURCE),
            db_ui::fixture::demo_snapshot(),
        ))
        .unwrap();
    store.set_model_cache(cache_dir.path());
    select(&mut h);
    h.invoke("db.explorer.refresh", Value::Null);
    wait(&mut h, "the prompt", asking);
    type_password(&mut h);
    wait(&mut h, "the connection", connected);
}

/// Cancelling the prompt connects nothing and says so in the tree, not in
/// a balloon.
#[gpui::test]
fn cancelling_the_prompt_does_not_connect(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, server, _consoles) = open(cx, &root, PasswordSave::Never);
    select(&mut h);
    h.invoke("db.explorer.refresh", Value::Null);
    wait(&mut h, "the prompt", asking);
    h.invoke("db.password-prompt.cancel", Value::Null);
    wait(&mut h, "the row", |h| {
        state(h)["explorer"]["tree"]["data_sources"][SOURCE]["error"]
            == "The password was not entered"
    });
    assert!(!connected(&mut h));
    assert_eq!(server.opened(Purpose::Meta), 0);
    assert_eq!(store.password(SOURCE), None);
    assert!(
        !notified(&mut h).contains("password"),
        "{}",
        notified(&mut h)
    );
}

/// A console of a "Never" data source asks too; Disconnect ends its
/// session, the Services row shows it, and the next statement asks again.
#[gpui::test]
fn a_console_asks_and_a_disconnect_ends_its_session(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, _server, _consoles) = open(cx, &root, PasswordSave::Never);
    select(&mut h);
    // Opening the console opens the data source: that asks first.
    h.invoke("db.console.new", json!({"source": SOURCE}));
    wait(&mut h, "the prompt", asking);
    type_password(&mut h);
    wait(&mut h, "the console", |h| !console(h).is_null());
    wait(&mut h, "the connection", connected);
    h.invoke("editor.type", json!({"text": "select 1;"}));

    for round in 1..=2 {
        h.invoke("db.console.execute", Value::Null);
        if round > 1 {
            // The disconnect took the password with it.
            wait(&mut h, "the prompt", asking);
            type_password(&mut h);
        }
        wait(&mut h, "the statement", |h| {
            let c = console(h);
            c["running"] == false
                && c["output"]
                    .to_string()
                    .matches("retrieved starting")
                    .count()
                    >= round
        });
        assert_eq!(console(&mut h)["disconnected"], false);
        assert!(connected(&mut h), "round {round}");

        h.invoke("db.explorer.disconnect", Value::Null);
        wait(&mut h, "the disconnect", |h| {
            console(h)["disconnected"] == true
        });
        assert!(!connected(&mut h));
        assert!(
            console(&mut h)["output"]
                .to_string()
                .contains("Disconnected")
        );
        wait(&mut h, "the password forgotten", |_| {
            store.password(SOURCE).is_none()
        });
    }
}

/// "Until restart" survives a disconnect; "Forever" saves the data source
/// with that choice and the password.
#[gpui::test]
fn the_choice_in_the_prompt_says_how_long_the_password_is_kept(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, _server, _consoles) = open(cx, &root, PasswordSave::Never);
    select(&mut h);

    h.invoke("db.explorer.refresh", Value::Null);
    wait(&mut h, "the prompt", asking);
    h.invoke("db.password-prompt.save", json!({"value": "until-restart"}));
    assert_eq!(state(&mut h)["prompt"]["save"], "until-restart");
    type_password(&mut h);
    wait(&mut h, "the connection", connected);
    h.invoke("db.explorer.disconnect", Value::Null);
    wait(&mut h, "the disconnect", |h| !connected(h));
    settle(&mut h);
    assert_eq!(store.password(SOURCE).as_deref(), Some("secret"));

    // Held: the next connect asks nothing.
    h.invoke("db.explorer.refresh", Value::Null);
    wait(&mut h, "the connection", connected);
    assert!(!asking(&mut h));
    h.invoke("db.explorer.disconnect", Value::Null);
    wait(&mut h, "the disconnect", |h| !connected(h));
    store.release_password(&root, SOURCE);

    assert_eq!(store.saves(), 0);
    h.invoke("db.explorer.refresh", Value::Null);
    wait(&mut h, "the prompt", asking);
    h.invoke("db.password-prompt.save", json!({"value": "forever"}));
    type_password(&mut h);
    wait(&mut h, "the save", |_| store.saves() == 1);
    wait(&mut h, "the connection", connected);
    let saved = store.sources(&root);
    assert_eq!(
        saved[0].profile.gui_options().password_save,
        PasswordSave::Forever
    );
    assert_eq!(store.password(SOURCE).as_deref(), Some("secret"));
    // The data source stays connected through the save.
    settle(&mut h);
    assert!(connected(&mut h));
}

/// A password the server refuses is forgotten, whatever it was kept for,
/// and the next connect asks again.
#[gpui::test]
fn a_refused_password_is_asked_again(cx: &mut TestAppContext) {
    let (_dir, root) = project();
    let (mut h, store, server, _consoles) = open(cx, &root, PasswordSave::UntilRestart);
    server.set_open(OpenBehavior::Fail(db_core::Error::new(
        db_core::ErrorKind::Auth,
        "password authentication failed",
    )));
    select(&mut h);

    for round in 1..=2 {
        h.invoke("db.explorer.refresh", Value::Null);
        wait(&mut h, "the prompt", asking);
        type_password(&mut h);
        wait(&mut h, "the refusal", |h| {
            state(h)["explorer"]["tree"]["data_sources"][SOURCE]["error"]
                == "password authentication failed"
        });
        wait(&mut h, "the password forgotten", |_| {
            store.password(SOURCE).is_none()
        });
        assert!(!connected(&mut h), "round {round}");
    }
}
