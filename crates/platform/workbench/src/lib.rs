//! The main window of Pyoxide IDE (ADR 0007): main toolbar in the title bar
//! area, tool window stripes, the project tool window, the editor area and
//! the status bar, plus application start-up and the environment switches
//! used by automated runs.
//!
//! The workbench knows no language. A product (for example the Python IDE)
//! describes itself with a [`Product`] and calls [`run`].

pub mod app;
pub mod chrome;
pub mod commands;
pub mod env;
pub mod file_types;
pub mod keymap;
pub mod project_panel;
mod shot;
pub mod workbench;

pub use app::{Product, StripeItem, init, open_main_window, run};
pub use file_types::{FileType, HighlighterFactory};
pub use workbench::Workbench;
