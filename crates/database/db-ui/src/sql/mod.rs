//! The SQL editor glue of the database module (ADR 0022): the SQL file
//! type with its highlighter, the dialect and scope of each file from the
//! project's mappings, and the toolbar above the editor of a SQL file
//! that shows and changes them.

use std::sync::Arc;

use ui::icons::IconName;
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

/// The SQL file type: it takes over the platform's plain `.sql` type with
/// the language id, comments and quotes of SQL, and the highlighter over
/// `hub`.
pub fn file_type(hub: Arc<Hub>) -> FileType {
    FileType {
        quotes: &['\'', '"'],
        language_id: Some(LANGUAGE_ID),
        highlighter: Some(Arc::new(SqlHighlighter::factory(hub.clone()))),
        decorator: Some(Arc::new(move |path| hub.decoration(path))),
        ..FileType::plain("SQL", &["sql"], IconName("fileTypes/sql")).with_comment("-- ")
    }
}
