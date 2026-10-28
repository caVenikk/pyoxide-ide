//! Tools > Database > Query Execution: the options of running statements in
//! a console that are kept for the application (`[database.query-execution]`
//! of the settings file).

use serde::{Deserialize, Serialize};
use settings::Section;

/// What running a statement checks and shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct QueryExecutionSettings {
    /// Ask for the values of the parameters of a statement before it runs
    /// (off: only when a value is missing).
    pub review_parameters: bool,
    /// Show how long a statement took after it, in the editor.
    pub show_execution_time: bool,
    /// Warn before a `DELETE` or `UPDATE` without `WHERE` runs.
    pub warn_unsafe_queries: bool,
}

impl Default for QueryExecutionSettings {
    fn default() -> Self {
        Self {
            review_parameters: true,
            show_execution_time: true,
            warn_unsafe_queries: true,
        }
    }
}

impl Section for QueryExecutionSettings {
    const KEY: &'static str = "database.query-execution";
}

/// The settings as stored.
pub(crate) fn get(cx: &gpui::App) -> QueryExecutionSettings {
    settings_ui::store::get(cx)
}

pub(crate) fn set(cx: &mut gpui::App, value: &QueryExecutionSettings) {
    settings_ui::store::set(cx, value);
}
