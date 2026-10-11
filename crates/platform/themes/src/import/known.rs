//! Identifiers that `parentTheme` and `parent_scheme` refer to. The theme
//! files themselves do not say which file an identifier belongs to, so this
//! table (observed on real theme files, see `docs/theme-formats.md`) names the
//! chain of the common ones. It carries no colours.

/// A UI theme that other themes name as their parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownTheme {
    /// The value used in `parentTheme`.
    pub id: &'static str,
    /// The `name` inside the theme file.
    pub name: &'static str,
    /// File name without extensions, to find the theme among available files.
    pub file_stem: &'static str,
    /// The parent of this theme, as an id.
    pub parent: Option<&'static str>,
    pub dark: bool,
}

pub const KNOWN_THEMES: &[KnownTheme] = &[
    KnownTheme {
        id: "Darcula",
        name: "Darcula",
        file_stem: "darcula",
        parent: None,
        dark: true,
    },
    KnownTheme {
        id: "IntelliJ",
        name: "IntelliJ",
        file_stem: "intellijlaf",
        parent: Some("Darcula"),
        dark: false,
    },
    KnownTheme {
        id: "ExperimentalDark",
        name: "Dark",
        file_stem: "expUI_dark",
        parent: Some("Darcula"),
        dark: true,
    },
    KnownTheme {
        id: "ExperimentalLight",
        name: "Light",
        file_stem: "expUI_light",
        parent: Some("IntelliJ"),
        dark: false,
    },
    KnownTheme {
        id: "ExperimentalLightWithLightHeader",
        name: "Light with Light Header",
        file_stem: "expUI_light_with_light_header",
        parent: Some("ExperimentalLight"),
        dark: false,
    },
    KnownTheme {
        id: "Islands Dark",
        name: "Islands Dark",
        file_stem: "ManyIslandsDark",
        parent: Some("ExperimentalDark"),
        dark: true,
    },
    KnownTheme {
        id: "Islands Light",
        name: "Islands Light",
        file_stem: "ManyIslandsLight",
        parent: Some("ExperimentalLightWithLightHeader"),
        dark: false,
    },
    KnownTheme {
        id: "Islands Darcula",
        name: "Islands Darcula",
        file_stem: "ManyIslandsDarcula",
        parent: Some("ExperimentalDark"),
        dark: true,
    },
    KnownTheme {
        id: "HighContrast",
        name: "High Contrast",
        file_stem: "HighContrast",
        parent: Some("Darcula"),
        dark: true,
    },
];

pub fn known_theme(id: &str) -> Option<&'static KnownTheme> {
    KNOWN_THEMES.iter().find(|t| t.id == id)
}

/// The ids from `id` up to the root, following the table. The slice stops at
/// an id that is not in the table.
pub fn known_chain(id: &str) -> Vec<&'static str> {
    let mut chain = Vec::new();
    let mut current = known_theme(id);
    while let Some(theme) = current {
        if chain.contains(&theme.id) {
            break;
        }
        chain.push(theme.id);
        current = theme.parent.and_then(known_theme);
    }
    chain
}

/// A base colour scheme that other schemes name in `parent_scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownScheme {
    pub name: &'static str,
    pub parent: Option<&'static str>,
    pub dark: bool,
}

pub const KNOWN_SCHEMES: &[KnownScheme] = &[
    KnownScheme {
        name: "Default",
        parent: None,
        dark: false,
    },
    KnownScheme {
        name: "Darcula",
        parent: Some("Default"),
        dark: true,
    },
];

pub fn known_scheme(name: &str) -> Option<&'static KnownScheme> {
    KNOWN_SCHEMES.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_follow_the_observed_inheritance() {
        assert_eq!(
            known_chain("Islands Dark"),
            ["Islands Dark", "ExperimentalDark", "Darcula"]
        );
        assert_eq!(
            known_chain("Islands Light"),
            [
                "Islands Light",
                "ExperimentalLightWithLightHeader",
                "ExperimentalLight",
                "IntelliJ",
                "Darcula"
            ]
        );
        assert!(known_chain("Unheard Of").is_empty());
    }

    #[test]
    fn every_parent_is_in_the_table_and_dark_flags_are_consistent_for_dark_roots() {
        for theme in KNOWN_THEMES {
            if let Some(parent) = theme.parent {
                assert!(known_theme(parent).is_some(), "{} -> {parent}", theme.id);
            }
        }
        assert!(known_theme("ExperimentalDark").unwrap().dark);
        assert!(!known_theme("ExperimentalLight").unwrap().dark);
    }

    #[test]
    fn base_schemes() {
        assert_eq!(known_scheme("Darcula").unwrap().parent, Some("Default"));
        assert!(known_scheme("Dracula").is_none());
    }
}
