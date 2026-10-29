//! The SQL editor glue of the database module (ADR 0022): the SQL file
//! type with its highlighter, the dialect and scope of each file from the
//! project's mappings, and the toolbar above the editor of a SQL file
//! that shows and changes them.

use std::sync::Arc;

use ui::icons::IconName;
use ui::menu::MenuItem;
use workbench::file_types::FileType;

pub mod highlight;
pub mod hub;
pub mod settings;
pub mod widget;

pub use highlight::{SqlHighlighter, SqlLexer};
pub use hub::{Hub, Kind, Source};
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
        decorator: Some(Arc::new({
            let hub = hub.clone();
            move |path| hub.decoration(path)
        })),
        context_menu: Some(Arc::new(move |path, _| context_menu(&hub, path))),
        key_context: Some(KEY_CONTEXT),
        ..FileType::plain("SQL", &["sql"], IconName("fileTypes/sql")).with_comment("-- ")
    }
}

/// The rows the editor's context menu gets for a SQL file attached to a data
/// source, a console or a file (the reference's `Explain Plan` group and
/// `Execute`, after Evaluate Expression): Explain Plan > Explain Plan,
/// Explain Analyze; Execute. A file with no data source has none.
fn context_menu(hub: &Hub, path: &std::path::Path) -> Vec<MenuItem> {
    if hub.resolve(path).data_source.is_none() {
        return Vec::new();
    }
    vec![
        MenuItem::submenu(
            "Explain Plan",
            vec![
                MenuItem::action("Explain Plan", crate::console::ExplainPlan),
                MenuItem::action("Explain Analyze", crate::console::ExplainAnalyze),
            ],
        )
        .icon(crate::icons::PLAN),
        MenuItem::action("Execute", crate::console::Execute).icon(IconName::RUN),
    ]
}
