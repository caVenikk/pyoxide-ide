//! The Settings window of Pyoxide IDE (ADR 0017) and the running
//! application's settings store.
//!
//! - [`store`]: the application settings (`settings::Store` in a global),
//!   typed sections, change events, background saves.
//! - [`page`]: the page registry and the [`SettingsPage`] trait. Areas add
//!   their pages with [`page::register`]; products through the workbench.
//! - [`open`]: the window (`SettingsEditor` in the reference's non-modal
//!   dialog), one per application.

pub mod commands;
pub mod page;
pub mod store;
mod window;

use std::rc::Rc;

use gpui::App;
use serde_json::{Value as Json, json};
use ui::command::CommandError;

pub use page::{CreatePage, PageContext, PageHandle, PageSpec, Scope, SettingsPage};
pub use window::{
    DIALOG_KEY, HEIGHT, KEY_CONTEXT, OpenRequest, REVERT, SPLIT, SettingsWindow, WIDTH, find, open,
    title,
};

/// Registers the commands; called once per application.
pub fn init(cx: &mut App) {
    ui::command::register(cx, commands::OPEN_COMMANDS);
    ui::command::register(cx, commands::COMMANDS);
    ui::command::register(cx, commands::QUERY_COMMANDS);
    ui::command::register_query(
        cx,
        "settings.state",
        Rc::new(|_, window, cx| {
            let Some(handle) = find(cx) else {
                return Ok(json!({"open": false}));
            };
            if window.window_handle() == handle.into() {
                let view = window
                    .root::<SettingsWindow>()
                    .flatten()
                    .ok_or_else(|| CommandError::new("not_available", "no Settings window"))?;
                return Ok(view.read(cx).state(window, cx));
            }
            handle
                .update(cx, |view, window, cx| view.state(window, cx))
                .map_err(|e| CommandError::new("not_available", e.to_string()))
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
        store::flush(cx);
        async {}
    })
    .detach();
}
