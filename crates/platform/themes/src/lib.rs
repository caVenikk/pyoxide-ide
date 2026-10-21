//! Theme model of Pyoxide IDE.
//!
//! A [`Theme`] assigns a value to every [`UiRole`] (flat colours of the
//! interface and the editor chrome), every [`TextRole`] (syntax tokens and
//! editor decorations with text attributes) and every [`Metric`] (layout
//! numbers), plus [`Fonts`]. Themes are stored as TOML ([`ThemeFile`]) with
//! optional `extends` inheritance; a [`Registry`] resolves them.
//!
//! The [`import`] module reads PyCharm colour schemes (`.icls`), UI themes
//! (`.theme.json`) and plugin archives that contain them, and reports what
//! could and could not be mapped. `docs/theme-mapping.md` lists the mapping.
//!
//! A [`Catalog`] holds the themes the IDE offers: the built-in ones and the
//! ones imported into the configuration folder, in the reference's groups.

pub mod builtin;
pub mod catalog;
mod color;
pub mod contrast;
pub mod icon_palette;
pub mod import;
mod model;
mod registry;
mod roles;
mod token_names;

pub use catalog::{Catalog, CatalogEntry, ThemeGroup};
pub use color::{ColorParseError, ColorRef, Rgba, contrast_ratio};
pub use model::{
    Controls, FORMAT_VERSION, FontPatch, Fonts, Metrics, TextStyle, TextStyles, Theme, ThemeFile,
    ThemeHeader, ThemeKind, UiColors, UnderlineStyle, skipped_warning,
};
pub use registry::{Registry, ThemeError};
pub use roles::{Metric, TextRole, TextRoleKind, UiRole};
