//! The Settings window of Pyoxide IDE (ADR 0017) and the running
//! application's settings store.
//!
//! - [`store`]: the application settings (`settings::Store` in a global),
//!   typed sections, change events, background saves.
//! - [`page`]: the page registry and the [`SettingsPage`] trait. Areas add
//!   their pages with [`page::register`]; products through the workbench.
//! - [`open`]: the window (`SettingsEditor` in the reference's non-modal
//!   dialog), one per application, with its [`search`] and the state it
//!   keeps between sessions ([`window_state`]).
//! - [`leave`]: Apply / Don't Save / Cancel when the project closes or the
//!   application quits with changes not applied.
//! - [`theme_switch`]: the theme and editor colour scheme in use, the
//!   theme switchers with their preview, and theme import; [`pages`] holds
//!   the pages that only edit sections of the store (Appearance).

pub mod commands;
mod history;
pub mod leave;
pub mod page;
pub mod pages;
pub mod search;
pub mod store;
pub mod theme_switch;
mod tree;
mod window;
pub mod window_state;

use std::rc::Rc;

use gpui::App;
use serde_json::{Value as Json, json};
use ui::command::CommandError;

pub use page::{CreatePage, PageContext, PageHandle, PageSpec, Scope, SettingsPage};
pub use tree::{SEARCH_CONTEXT, TREE_CONTEXT};
pub use window::{
    DIALOG_KEY, HEIGHT, KEY_CONTEXT, OpenRequest, PROJECT_MARK, REVERT, SPLIT, SettingsWindow,
    WIDTH, find, open, title,
};

/// Registers the commands; called once per application.
pub fn init(cx: &mut App) {
    ui::command::register(cx, commands::OPEN_COMMANDS);
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::QUERY_COMMANDS);
    ui::command::register(cx, commands::THEME_COMMANDS);
    theme_switch::init(cx);
    ui::command::register_query(
        cx,
        "settings.state",
        Rc::new(|_, window, cx| {
            let mut state = match find(cx) {
                None => json!({"open": false}),
                Some(handle) if window.window_handle() == handle.into() => {
                    let view = window
                        .root::<SettingsWindow>()
                        .flatten()
                        .ok_or_else(|| CommandError::new("not_available", "no Settings window"))?;
                    view.read(cx).state(window, cx)
                }
                Some(handle) => handle
                    .update(cx, |view, window, cx| view.state(window, cx))
                    .map_err(|e| CommandError::new("not_available", e.to_string()))?,
            };
            state["appearance"] = theme_switch::state_json(cx);
            Ok(state)
        }),
    );
    ui::command::register_query(
        cx,
        "settings.get",
        Rc::new(|args, _, cx| {
            let key = args.get("key").and_then(Json::as_str).unwrap_or("");
            store::get_key(cx, key)
                .ok_or_else(|| CommandError::new("not_found", format!("unknown setting `{key}`")))
        }),
    );
    ui::command::register_query(
        cx,
        "settings.set",
        Rc::new(|args, _, cx| {
            let key = args
                .get("key")
                .and_then(Json::as_str)
                .ok_or_else(|| CommandError::invalid_args("settings.set: missing `key`"))?;
            let value = args
                .get("value")
                .cloned()
                .ok_or_else(|| CommandError::invalid_args("settings.set: missing `value`"))?;
            store::set_key(cx, key, value)
                .map_err(|e| CommandError::invalid_args(e.to_string()))?;
            Ok(Json::Null)
        }),
    );
    // Changes not saved yet are written before the application quits.
    cx.on_app_quit(|cx| {
        if let Some(window) = find(cx) {
            let _ = window.update(cx, |view, _, cx| view.remember_now(cx));
        }
        store::flush(cx);
        window_state::flush(cx);
        async {}
    })
    .detach();
}
