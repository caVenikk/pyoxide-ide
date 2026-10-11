use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

/// Where a key of the imported material was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeySource {
    /// `ui` section of a `.theme.json`.
    UiTheme,
    /// Other sections of a `.theme.json` (icons, ...).
    UiThemeSection,
    /// `<colors>` of a colour scheme.
    SchemeColor,
    /// `<attributes>` of a colour scheme.
    SchemeAttribute,
    /// Font and other options directly under `<scheme>`.
    SchemeOption,
}

impl KeySource {
    pub fn label(self) -> &'static str {
        match self {
            KeySource::UiTheme => "ui",
            KeySource::UiThemeSection => "theme section",
            KeySource::SchemeColor => "scheme color",
            KeySource::SchemeAttribute => "scheme attribute",
            KeySource::SchemeOption => "scheme option",
        }
    }
}

/// What kind of file the import started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    #[default]
    UiTheme,
    Scheme,
}

/// A key that was read and put into the theme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappedKey {
    pub source: KeySource,
    pub key: String,
    /// The theme token it went to: `ui.accent`, `syntax.keyword`,
    /// `metrics.row-height` or `font.editor-size`.
    pub target: String,
    /// False when a key listed earlier for the same token won, so this one
    /// was read but not used.
    pub used: bool,
}

/// A key the mapping has no place for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmappedKey {
    pub source: KeySource,
    pub key: String,
    /// How many times it occurs (a key can be set by the theme and by its
    /// parents; a section counts its entries).
    pub count: usize,
}

/// A token that was filled in a lossy way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approximation {
    pub key: String,
    pub target: String,
    pub note: String,
}

/// A token that no key supplied, so the base theme's value is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Defaulted {
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum WarningKind {
    /// `parentTheme` names an id that this importer does not know.
    UnknownParentTheme { id: String },
    /// `parentTheme` is a known id but its file is not among the inputs.
    ParentThemeUnavailable { id: String, chain: Vec<String> },
    /// `parent_scheme` names a scheme that is not among the inputs.
    ParentSchemeUnavailable { name: String, known: bool },
    /// `editorScheme` cannot be found.
    SchemeNotFound { reference: String },
    /// A value that should be a colour or number is not.
    InvalidValue { key: String, value: String },
    /// A palette name that is not defined.
    UnresolvedPaletteName { key: String, name: String },
    /// The inheritance chain loops.
    InheritanceCycle { id: String },
    /// Values supplied by the base theme because the sources omit them.
    MissingBaseValues { count: usize },
    /// A file or archive entry was skipped.
    Skipped { path: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    pub level: Level,
    #[serde(flatten)]
    pub kind: WarningKind,
    pub message: String,
}

/// Counts for a quick overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Summary {
    /// Keys that supplied a value.
    pub mapped: usize,
    /// Keys that were read but lost against an earlier key of the same token.
    pub shadowed: usize,
    /// Distinct keys without a place in the theme.
    pub unmapped_keys: usize,
    /// Occurrences of those keys.
    pub unmapped_total: usize,
    pub approximations: usize,
    pub defaulted: usize,
    pub warnings: usize,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub name: String,
    pub kind: SourceKind,
    /// Files and archive entries the data came from.
    pub sources: Vec<String>,
    /// Built-in theme that supplied everything the sources omit.
    pub base: String,
    pub mapped: Vec<MappedKey>,
    pub unmapped: Vec<UnmappedKey>,
    pub approximations: Vec<Approximation>,
    pub defaulted: Vec<Defaulted>,
    pub warnings: Vec<Warning>,
}

impl ImportReport {
    pub(crate) fn warn(&mut self, level: Level, kind: WarningKind, message: String) {
        self.warnings.push(Warning {
            level,
            kind,
            message,
        });
    }

    pub fn summary(&self) -> Summary {
        Summary {
            mapped: self.mapped.iter().filter(|m| m.used).count(),
            shadowed: self.mapped.iter().filter(|m| !m.used).count(),
            unmapped_keys: self.unmapped.len(),
            unmapped_total: self.unmapped.iter().map(|u| u.count).sum(),
            approximations: self.approximations.len(),
            defaulted: self.defaulted.len(),
            warnings: self
                .warnings
                .iter()
                .filter(|w| w.level == Level::Warning)
                .count(),
        }
    }

    /// Unmapped keys grouped by their first segment (`Button.arc` is in
    /// `Button`, `PY.BUILTIN_NAME` in `PY`, `ERRORS_ATTRIBUTES` in `ERRORS`),
    /// largest group first.
    pub fn unmapped_groups(&self) -> Vec<(String, usize)> {
        let mut groups: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.unmapped {
            *groups.entry(group_of(&entry.key)).or_default() += entry.count;
        }
        let mut out: Vec<_> = groups.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// A human readable report. With `verbose` every key is listed.
    pub fn render(&self, verbose: bool) -> String {
        let s = self.summary();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} ({}, base {})",
            self.name,
            match self.kind {
                SourceKind::UiTheme => "UI theme",
                SourceKind::Scheme => "colour scheme",
            },
            self.base
        );
        for source in &self.sources {
            let _ = writeln!(out, "  source: {source}");
        }
        let _ = writeln!(
            out,
            "  mapped {} keys ({} shadowed), unmapped {} keys ({} occurrences), {} approximations, {} filled from base, {} warnings",
            s.mapped, s.shadowed, s.unmapped_keys, s.unmapped_total, s.approximations, s.defaulted, s.warnings
        );
        for w in &self.warnings {
            let tag = match w.level {
                Level::Info => "note",
                Level::Warning => "warning",
            };
            let _ = writeln!(out, "  {tag}: {}", w.message);
        }
        let groups = self.unmapped_groups();
        if !groups.is_empty() {
            let shown: Vec<String> = groups
                .iter()
                .take(if verbose { usize::MAX } else { 8 })
                .map(|(g, n)| format!("{g} {n}"))
                .collect();
            let more = groups.len().saturating_sub(shown.len());
            let _ = writeln!(
                out,
                "  unmapped by group: {}{}",
                shown.join(", "),
                if more > 0 {
                    format!(", and {more} more groups")
                } else {
                    String::new()
                }
            );
        }
        if verbose {
            for m in &self.mapped {
                let _ = writeln!(
                    out,
                    "  map   {:<16} {:<48} -> {}{}",
                    m.source.label(),
                    m.key,
                    m.target,
                    if m.used { "" } else { " (shadowed)" }
                );
            }
            for a in &self.approximations {
                let _ = writeln!(out, "  approx {} -> {}: {}", a.key, a.target, a.note);
            }
            for d in &self.defaulted {
                let _ = writeln!(out, "  base  {}", d.target);
            }
            for u in &self.unmapped {
                let _ = writeln!(
                    out,
                    "  skip  {:<16} {} x{}",
                    u.source.label(),
                    u.key,
                    u.count
                );
            }
        }
        out
    }
}

fn group_of(key: &str) -> String {
    let first = key.split(['.', '_']).next().unwrap_or(key);
    if first.is_empty() { key } else { first }.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unmapped(key: &str, count: usize) -> UnmappedKey {
        UnmappedKey {
            source: KeySource::SchemeAttribute,
            key: key.into(),
            count,
        }
    }

    #[test]
    fn groups_split_on_dots_and_underscores() {
        let report = ImportReport {
            unmapped: vec![
                unmapped("PY.X", 1),
                unmapped("PY.Y", 2),
                unmapped("ERRORS_ATTRIBUTES", 1),
                unmapped("ERRORS_OTHER", 1),
                unmapped("Button.arc", 4),
            ],
            ..ImportReport::default()
        };
        assert_eq!(
            report.unmapped_groups(),
            [
                ("Button".to_owned(), 4),
                ("PY".to_owned(), 3),
                ("ERRORS".to_owned(), 2)
            ]
        );
        let s = report.summary();
        assert_eq!((s.unmapped_keys, s.unmapped_total), (5, 9));
    }

    #[test]
    fn summary_counts_levels() {
        let mut report = ImportReport::default();
        report.warn(
            Level::Info,
            WarningKind::MissingBaseValues { count: 1 },
            "x".into(),
        );
        report.warn(
            Level::Warning,
            WarningKind::UnknownParentTheme { id: "a".into() },
            "y".into(),
        );
        assert_eq!(report.summary().warnings, 1);
        assert!(report.render(false).contains("warning: y"));
        assert!(report.render(false).contains("note: x"));
    }
}
