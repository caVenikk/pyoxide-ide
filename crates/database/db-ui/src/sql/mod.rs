//! The SQL editor glue of the database module (ADR 0022): the SQL file
//! type with its highlighter, the dialect and scope of each file from the
//! project's mappings, and the toolbar above the editor of a SQL file
//! that shows and changes them.

use std::sync::Arc;

use ui::icons::IconName;
use workbench::file_types::FileType;

pub mod highlight;
pub mod hub;
pub mod intel;
mod models;
pub mod settings;
pub mod widget;

pub use highlight::{SqlHighlighter, SqlLexer};
pub use hub::{ConsoleBinding, Hub, Kind, Source};
pub use intel::{HubScope, SqlIntelProvider};
pub use widget::{dialect_title, is_sql_file};

/// The language id the SQL language services serve.
pub const LANGUAGE_ID: &str = "sql";

/// The word the editors of SQL files add to their key context: Execute is
/// bound to it (`keymaps/*/console.toml`), in no other editor.
pub const KEY_CONTEXT: &str = "SqlFile";

/// The SQL file type: it takes over the platform's plain `.sql` type with
/// the language id, comments and quotes of SQL, and the highlighter over
/// `hub`.
pub fn file_type(hub: Arc<Hub>) -> FileType {
    FileType {
        quotes: &['\'', '"'],
        language_id: Some(LANGUAGE_ID),
        highlighter: Some(Arc::new(SqlHighlighter::factory(hub.clone()))),
        decorator: Some(Arc::new(move |path| hub.decoration(path))),
        key_context: Some(KEY_CONTEXT),
        ..FileType::plain("SQL", &["sql"], IconName("fileTypes/sql")).with_comment("-- ")
    }
}

/// A data source's model was loaded or refreshed in the Database tool
/// window: the SQL files attached to it resolve through it.
pub(crate) fn publish_model(
    cx: &gpui::App,
    root: &std::path::Path,
    source: &str,
    model: &Arc<db_model::Model>,
) {
    if model.is_empty() {
        return;
    }
    if let Some(global) = cx.try_global::<widget::SqlGlobal>() {
        global.0.publish_model(root, source, model.clone());
    }
}
