use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::color::{ColorRef, Rgba};
use crate::model::{
    FontPatch, Metrics, TextStyle, TextStyles, Theme, ThemeFile, ThemeKind, UiColors,
};
use crate::roles::{Metric, TextRole, UiRole};

/// Longest `extends` chain accepted.
const MAX_DEPTH: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("{origin}: {message}")]
    Parse { origin: String, message: String },
    #[error("{origin}: theme file format {format} is newer than this version understands")]
    UnsupportedFormat { origin: String, format: u32 },
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("theme file has no `theme.id`")]
    MissingId,
    #[error("unknown theme `{0}`")]
    UnknownTheme(String),
    #[error("`extends` cycle: {}", .0.join(" -> "))]
    ExtendsCycle(Vec<String>),
    #[error("`extends` chain of `{0}` is deeper than {MAX_DEPTH}")]
    ExtendsTooDeep(String),
    #[error("theme `{theme}`: `{role}` refers to the unknown palette entry `{name}`")]
    UnknownPalette {
        theme: String,
        role: String,
        name: String,
    },
    #[error("theme `{theme}` is incomplete, missing: {}", missing.join(", "))]
    Incomplete { theme: String, missing: Vec<String> },
}

/// Theme files by id, with `extends` resolution.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    files: BTreeMap<String, ThemeFile>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry holding the themes that ship with the IDE.
    pub fn with_builtin() -> Self {
        let mut registry = Self::new();
        for file in crate::builtin::files() {
            registry
                .insert(file)
                .expect("built-in themes have ids and are unique");
        }
        registry
    }

    /// Adds or replaces a theme file under its `theme.id`.
    pub fn insert(&mut self, file: ThemeFile) -> Result<(), ThemeError> {
        if file.theme.id.trim().is_empty() {
            return Err(ThemeError::MissingId);
        }
        self.files.insert(file.theme.id.clone(), file);
        Ok(())
    }

    pub fn insert_toml(&mut self, text: &str, origin: &str) -> Result<String, ThemeError> {
        let file = ThemeFile::from_toml(text, origin)?;
        let id = file.theme.id.clone();
        self.insert(file)?;
        Ok(id)
    }

    /// Loads every `*.toml` file of a directory (not recursive). Returns the
    /// ids that were added.
    pub fn load_dir(&mut self, dir: &Path) -> Result<Vec<String>, ThemeError> {
        let io = |source| ThemeError::Io {
            path: dir.to_owned(),
            source,
        };
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        let mut ids = Vec::new();
        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|source| ThemeError::Io {
                path: path.clone(),
                source,
            })?;
            ids.push(self.insert_toml(&text, &path.display().to_string())?);
        }
        Ok(ids)
    }

    pub fn get(&self, id: &str) -> Option<&ThemeFile> {
        self.files.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.files.contains_key(id)
    }

    /// Resolves a registered theme: follows `extends` and fills every role.
    pub fn resolve(&self, id: &str) -> Result<Theme, ThemeError> {
        let file = self
            .files
            .get(id)
            .ok_or_else(|| ThemeError::UnknownTheme(id.to_owned()))?;
        self.resolve_file(file)
    }

    /// Resolves a file that need not be registered; its `extends` refers to
    /// registered themes.
    pub fn resolve_file(&self, file: &ThemeFile) -> Result<Theme, ThemeError> {
        let mut chain: Vec<&ThemeFile> = vec![file];
        let mut seen = vec![file.theme.id.clone()];
        let mut current = file;
        while let Some(parent_id) = current.theme.extends.as_deref() {
            if seen.iter().any(|s| s == parent_id) {
                seen.push(parent_id.to_owned());
                return Err(ThemeError::ExtendsCycle(seen));
            }
            if chain.len() >= MAX_DEPTH {
                return Err(ThemeError::ExtendsTooDeep(file.theme.id.clone()));
            }
            let parent = self
                .files
                .get(parent_id)
                .ok_or_else(|| ThemeError::UnknownTheme(parent_id.to_owned()))?;
            seen.push(parent_id.to_owned());
            chain.push(parent);
            current = parent;
        }
        merge_chain(&chain)
    }
}

fn merge_chain(chain: &[&ThemeFile]) -> Result<Theme, ThemeError> {
    // `chain` is leaf first; apply root first so the leaf wins.
    let leaf = chain[0];
    let theme_id = leaf.theme.id.clone();

    let mut palette: BTreeMap<&str, Rgba> = BTreeMap::new();
    let mut ui: BTreeMap<UiRole, &ColorRef> = BTreeMap::new();
    let mut syntax: BTreeMap<TextRole, TextStyle<ColorRef>> = BTreeMap::new();
    let mut font = FontPatch::default();
    let mut metrics: BTreeMap<Metric, f32> = BTreeMap::new();
    for file in chain.iter().rev() {
        palette.extend(file.palette.iter().map(|(k, v)| (k.as_str(), *v)));
        ui.extend(file.ui.iter().map(|(k, v)| (*k, v)));
        for (role, style) in &file.syntax {
            let merged = match syntax.get(role) {
                Some(existing) => existing.overlay(style),
                None => style.clone(),
            };
            syntax.insert(*role, merged);
        }
        font = font.overlay(&file.font);
        metrics.extend(file.metrics.iter().map(|(k, v)| (*k, *v)));
    }
    let kind = chain.iter().find_map(|f| f.theme.kind);

    let mut missing: Vec<String> = Vec::new();
    if kind.is_none() {
        missing.push("theme.kind".to_owned());
    }
    let resolve = |role: &str, color: &ColorRef| -> Result<Rgba, ThemeError> {
        match color {
            ColorRef::Rgba(c) => Ok(*c),
            ColorRef::Palette(name) => {
                palette
                    .get(name.as_str())
                    .copied()
                    .ok_or_else(|| ThemeError::UnknownPalette {
                        theme: theme_id.clone(),
                        role: role.to_owned(),
                        name: name.clone(),
                    })
            }
        }
    };

    let mut colors = UiColors::filled(Rgba::BLACK);
    for &role in UiRole::ALL {
        match ui.get(&role) {
            Some(color) => colors[role] = resolve(role.name(), color)?,
            None => missing.push(format!("ui.{role}")),
        }
    }
    let mut text = TextStyles::default();
    for (role, style) in &syntax {
        text[*role] = style.map_colors(|c| resolve(role.name(), c))?;
    }
    let mut numbers = Metrics::filled(0.0);
    for &metric in Metric::ALL {
        match metrics.get(&metric) {
            Some(value) => numbers[metric] = *value,
            None => missing.push(format!("metrics.{metric}")),
        }
    }
    missing.extend(font.missing().into_iter().map(str::to_owned));
    if !missing.is_empty() {
        return Err(ThemeError::Incomplete {
            theme: theme_id,
            missing,
        });
    }
    Ok(Theme {
        id: leaf.theme.id.clone(),
        name: leaf.theme.name.clone(),
        kind: kind.unwrap_or(ThemeKind::Dark),
        author: leaf.theme.author.clone(),
        description: leaf.theme.description.clone(),
        ui: colors,
        text,
        fonts: font.complete().expect("checked by `missing`"),
        metrics: numbers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(extends: &str) -> ThemeFile {
        let mut file = ThemeFile::default();
        file.theme.id = "leaf".into();
        file.theme.name = "Leaf".into();
        file.theme.extends = Some(extends.into());
        file
    }

    #[test]
    fn child_overrides_parent_and_inherits_the_rest() {
        let registry = Registry::with_builtin();
        let base = registry.resolve("pyoxide-dark").unwrap();
        let mut file = leaf("pyoxide-dark");
        file.palette.insert("mine".into(), Rgba::rgb(1, 2, 3));
        file.ui
            .insert(UiRole::Accent, ColorRef::Palette("mine".into()));
        file.syntax.insert(
            TextRole::Keyword,
            TextStyle {
                fg: Some(ColorRef::Palette("mine".into())),
                ..TextStyle::default()
            },
        );
        file.metrics.insert(Metric::RowHeight, 40.0);
        let theme = registry.resolve_file(&file).unwrap();
        assert_eq!(theme.id, "leaf");
        assert_eq!(theme.kind, ThemeKind::Dark);
        assert_eq!(theme.ui[UiRole::Accent], Rgba::rgb(1, 2, 3));
        assert_eq!(theme.ui[UiRole::Surface], base.ui[UiRole::Surface]);
        assert_eq!(theme.text[TextRole::Keyword].fg, Some(Rgba::rgb(1, 2, 3)));
        // Fields the child does not mention survive from the parent.
        assert_eq!(
            theme.text[TextRole::Keyword].bold,
            base.text[TextRole::Keyword].bold
        );
        assert_eq!(theme.metrics[Metric::RowHeight], 40.0);
        assert_eq!(theme.fonts, base.fonts);
    }

    #[test]
    fn palette_of_the_parent_is_visible_to_the_child() {
        let mut registry = Registry::with_builtin();
        let mut parent = leaf("pyoxide-dark");
        parent.theme.id = "parent".into();
        parent.palette.insert("shared".into(), Rgba::rgb(9, 8, 7));
        registry.insert(parent).unwrap();
        let mut child = leaf("parent");
        child
            .ui
            .insert(UiRole::Info, ColorRef::Palette("shared".into()));
        let theme = registry.resolve_file(&child).unwrap();
        assert_eq!(theme.ui[UiRole::Info], Rgba::rgb(9, 8, 7));
    }

    #[test]
    fn errors_are_specific() {
        let mut registry = Registry::with_builtin();
        assert!(matches!(
            registry.resolve("nope"),
            Err(ThemeError::UnknownTheme(_))
        ));
        assert!(matches!(
            registry.resolve_file(&leaf("nope")),
            Err(ThemeError::UnknownTheme(_))
        ));

        let mut a = leaf("b");
        a.theme.id = "a".into();
        let mut b = leaf("a");
        b.theme.id = "b".into();
        registry.insert(a).unwrap();
        registry.insert(b).unwrap();
        assert!(matches!(
            registry.resolve("a"),
            Err(ThemeError::ExtendsCycle(_))
        ));

        let mut bad = leaf("pyoxide-dark");
        bad.ui
            .insert(UiRole::Accent, ColorRef::Palette("ghost".into()));
        assert!(matches!(
            registry.resolve_file(&bad),
            Err(ThemeError::UnknownPalette { .. })
        ));

        let mut orphan = ThemeFile::default();
        orphan.theme.id = "orphan".into();
        orphan.theme.name = "Orphan".into();
        match registry.resolve_file(&orphan) {
            Err(ThemeError::Incomplete { missing, .. }) => {
                assert!(missing.iter().any(|m| m == "theme.kind"));
                assert!(missing.iter().any(|m| m == "ui.surface"));
                assert!(missing.iter().any(|m| m == "metrics.row-height"));
                assert!(missing.iter().any(|m| m == "font.editor-size"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            registry.insert(ThemeFile::default()),
            Err(ThemeError::MissingId)
        ));
    }

    #[test]
    fn load_dir_reads_toml_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = leaf("pyoxide-light");
        file.theme.id = "from-disk".into();
        std::fs::write(dir.path().join("a.toml"), file.to_toml()).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let mut registry = Registry::with_builtin();
        assert_eq!(registry.load_dir(dir.path()).unwrap(), vec!["from-disk"]);
        assert_eq!(
            registry.resolve("from-disk").unwrap().kind,
            ThemeKind::Light
        );
    }
}
