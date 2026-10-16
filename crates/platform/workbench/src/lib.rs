//! The main window of Pyoxide IDE (ADR 0007): main toolbar in the title bar
//! area, tool window stripes, the project tool window, the editor area and
//! the status bar, plus application start-up and the environment switches
//! used by automated runs.
//!
//! The workbench knows no language. A product (for example the Python IDE)
//! describes itself with a [`Product`] and calls [`run`].

pub mod app;
mod bench;
pub mod chrome;
pub mod commands;
pub mod dialog;
pub mod editor_settings;
pub mod env;
pub mod file_types;
pub mod find_in_files;
pub mod keymap;
pub mod navigation;
pub mod popup;
pub mod project_files;
pub mod project_panel;
pub mod recent_files;
pub mod run;
pub mod search_everywhere;
pub mod settings;
mod shot;
pub mod workbench;

pub use app::{NavigationHooks, Product, StatusWidget, StripeItem, init, open_main_window, run};
pub use code_intel_ui::{CodeIntelHook, CodeIntelProvider};
pub use file_types::{FileAssociation, FileType, HighlighterFactory};
pub use project::AutoExclusions;
pub use project_panel::{NewItem, NewItemKind, NewTemplate, RootKind, TreeHooks};
pub use run::{FieldText, RunHook, RunProvider, RunRequest, RunSpec};
pub use settings::{SaveSettings, SyncSettings};
pub use workbench::{Workbench, WorkbenchEvent};
