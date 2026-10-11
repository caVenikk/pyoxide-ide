//! Importers for PyCharm colour schemes (`.icls` and scheme XML), UI themes
//! (`.theme.json`) and the plugin archives that contain them.
//!
//! Only data files are read. The result of an import is a Pyoxide
//! [`Theme`](crate::Theme) and an [`ImportReport`] that lists what was mapped,
//! what was not, what was approximated and what the built-in base theme had
//! to supply. The mapping is documented in `docs/theme-mapping.md`.
//!
//! ```no_run
//! use std::path::Path;
//!
//! let imported = themes::import::import_path(Path::new("MyTheme.jar"), &[]).unwrap();
//! for item in &imported {
//!     println!("{}", item.report.render(false));
//!     let toml = item.theme.to_file().to_toml();
//!     # let _ = toml;
//! }
//! ```

mod archive;
mod convert;
mod known;
mod library;
mod mapping;
mod report;
mod scheme;
mod ui_theme;
mod xml;

pub use archive::ArchiveSummary;
pub use convert::Imported;
pub use known::{KNOWN_SCHEMES, KNOWN_THEMES, KnownScheme, KnownTheme};
pub use library::{ImportError, Library, Skipped, import_path};
pub use report::{
    Approximation, Defaulted, ImportReport, KeySource, Level, MappedKey, SourceKind, Summary,
    UnmappedKey, Warning, WarningKind,
};
pub use scheme::{Attribute, Scheme, SchemeFonts};
pub use ui_theme::UiThemeData;
