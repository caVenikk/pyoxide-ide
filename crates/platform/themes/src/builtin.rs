//! The themes that ship with the IDE. Their colours are original to this
//! project and are not derived from any other product.

use crate::model::{Theme, ThemeFile, ThemeKind};
use crate::registry::Registry;

pub const DARK_ID: &str = "pyoxide-dark";
pub const LIGHT_ID: &str = "pyoxide-light";

const DARK_TOML: &str = include_str!("../themes/pyoxide-dark.toml");
const LIGHT_TOML: &str = include_str!("../themes/pyoxide-light.toml");

/// The built-in theme files, dark first.
pub fn files() -> Vec<ThemeFile> {
    vec![
        ThemeFile::from_toml(DARK_TOML, "pyoxide-dark.toml").expect("built-in dark theme parses"),
        ThemeFile::from_toml(LIGHT_TOML, "pyoxide-light.toml")
            .expect("built-in light theme parses"),
    ]
}

/// Id of the built-in theme for a kind.
pub const fn id_for(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => DARK_ID,
        ThemeKind::Light => LIGHT_ID,
    }
}

/// The built-in theme for a kind, fully resolved.
pub fn theme(kind: ThemeKind) -> Theme {
    Registry::with_builtin()
        .resolve(id_for(kind))
        .expect("built-in themes are complete")
}

pub fn dark() -> Theme {
    theme(ThemeKind::Dark)
}

pub fn light() -> Theme {
    theme(ThemeKind::Light)
}
