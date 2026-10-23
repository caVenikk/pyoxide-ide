//! The themes that ship with the IDE.
//!
//! - `islands-dark` and `islands-light`, the defaults: the colours of the New
//!   UI with islands, generated from the IntelliJ Platform Community theme
//!   files (Apache-2.0, see NOTICE) by the `builtin_islands` example.
//! - `pyoxide-dark` and `pyoxide-light`: colours original to this project.
//!   They define every role and are the base of an import (also of the
//!   islands themes), so metrics and fonts are kept here. An import whose
//!   `parentTheme` chain names an islands theme takes that theme as its base.

use crate::model::{Theme, ThemeFile, ThemeKind};
use crate::registry::Registry;

pub const DARK_ID: &str = "pyoxide-dark";
pub const LIGHT_ID: &str = "pyoxide-light";
pub const ISLANDS_DARK_ID: &str = "islands-dark";
pub const ISLANDS_LIGHT_ID: &str = "islands-light";

/// The ids of the built-in themes.
pub(crate) const IDS: [&str; 4] = [DARK_ID, LIGHT_ID, ISLANDS_DARK_ID, ISLANDS_LIGHT_ID];

const DARK_TOML: &str = include_str!("../themes/pyoxide-dark.toml");
const LIGHT_TOML: &str = include_str!("../themes/pyoxide-light.toml");
const ISLANDS_DARK_TOML: &str = include_str!("../themes/islands-dark.toml");
const ISLANDS_LIGHT_TOML: &str = include_str!("../themes/islands-light.toml");

/// The built-in theme files: the base themes, then the defaults.
pub fn files() -> Vec<ThemeFile> {
    [
        (DARK_TOML, "pyoxide-dark.toml"),
        (LIGHT_TOML, "pyoxide-light.toml"),
        (ISLANDS_DARK_TOML, "islands-dark.toml"),
        (ISLANDS_LIGHT_TOML, "islands-light.toml"),
    ]
    .into_iter()
    .map(|(text, origin)| ThemeFile::from_toml(text, origin).expect("built-in theme parses"))
    .collect()
}

/// Id of the base theme for a kind (the base of imports).
pub const fn id_for(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => DARK_ID,
        ThemeKind::Light => LIGHT_ID,
    }
}

/// Id of the default theme for a kind.
pub const fn default_id_for(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => ISLANDS_DARK_ID,
        ThemeKind::Light => ISLANDS_LIGHT_ID,
    }
}

/// Id of the base theme of an import for a kind: the islands theme when the
/// imported theme descends from one, else the original base theme.
pub const fn import_id_for(kind: ThemeKind, islands: bool) -> &'static str {
    if islands {
        default_id_for(kind)
    } else {
        id_for(kind)
    }
}

/// The base theme of an import, fully resolved (see [`import_id_for`]).
pub fn import_base(kind: ThemeKind, islands: bool) -> Theme {
    resolve(import_id_for(kind, islands))
}

/// The base theme for a kind, fully resolved.
pub fn theme(kind: ThemeKind) -> Theme {
    resolve(id_for(kind))
}

/// The default theme for a kind, fully resolved.
pub fn default_theme(kind: ThemeKind) -> Theme {
    resolve(default_id_for(kind))
}

fn resolve(id: &str) -> Theme {
    Registry::with_builtin()
        .resolve(id)
        .expect("built-in themes are complete")
}

pub fn dark() -> Theme {
    theme(ThemeKind::Dark)
}

pub fn light() -> Theme {
    theme(ThemeKind::Light)
}
