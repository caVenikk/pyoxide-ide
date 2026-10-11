use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

use crate::color::{ColorRef, Rgba};
use crate::roles::{Metric, TextRole, UiRole};

/// Whether a theme is meant for a dark or a light background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Dark,
    Light,
}

/// Line decoration under text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnderlineStyle {
    None,
    Single,
    Double,
    Wavy,
    Dotted,
    Dashed,
    Thick,
}

/// Text attributes of a [`TextRole`]. Every field is optional: `None` means
/// "not set here", so a style can override only part of a parent's style and
/// a plain style leaves the editor defaults alone.
///
/// Effects of the editor colour scheme model map as follows: boxed text is
/// `border`, underline variants are `underline`, strikeout is
/// `strikethrough`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct TextStyle<C = Rgba> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<C>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<C>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<UnderlineStyle>,
    /// Colour of the underline; the text colour when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<C>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,
    /// Colour of a box drawn around the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<C>,
}

impl<C> Default for TextStyle<C> {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: None,
            italic: None,
            underline: None,
            underline_color: None,
            strikethrough: None,
            border: None,
        }
    }
}

impl<C: Clone> TextStyle<C> {
    /// Fields set in `over` win; unset ones are taken from `self`.
    pub fn overlay(&self, over: &TextStyle<C>) -> TextStyle<C> {
        TextStyle {
            fg: over.fg.clone().or_else(|| self.fg.clone()),
            bg: over.bg.clone().or_else(|| self.bg.clone()),
            bold: over.bold.or(self.bold),
            italic: over.italic.or(self.italic),
            underline: over.underline.or(self.underline),
            underline_color: over
                .underline_color
                .clone()
                .or_else(|| self.underline_color.clone()),
            strikethrough: over.strikethrough.or(self.strikethrough),
            border: over.border.clone().or_else(|| self.border.clone()),
        }
    }

    pub fn map_colors<D, E>(
        &self,
        mut f: impl FnMut(&C) -> Result<D, E>,
    ) -> Result<TextStyle<D>, E> {
        let mut conv = |c: &Option<C>| c.as_ref().map(&mut f).transpose();
        Ok(TextStyle {
            fg: conv(&self.fg)?,
            bg: conv(&self.bg)?,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            underline_color: conv(&self.underline_color)?,
            strikethrough: self.strikethrough,
            border: conv(&self.border)?,
        })
    }
}

impl<C> TextStyle<C> {
    /// True when nothing is set.
    pub fn is_plain(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.underline_color.is_none()
            && self.strikethrough.is_none()
            && self.border.is_none()
    }
}

impl TextStyle<Rgba> {
    pub fn is_bold(&self) -> bool {
        self.bold.unwrap_or(false)
    }

    pub fn is_italic(&self) -> bool {
        self.italic.unwrap_or(false)
    }

    pub fn underline_style(&self) -> UnderlineStyle {
        self.underline.unwrap_or(UnderlineStyle::None)
    }

    pub fn is_strikethrough(&self) -> bool {
        self.strikethrough.unwrap_or(false)
    }

    /// Colour of the underline: its own colour, else the text colour.
    pub fn underline_colour(&self) -> Option<Rgba> {
        self.underline_color.or(self.fg)
    }

    pub fn with_fg(fg: Rgba) -> Self {
        Self {
            fg: Some(fg),
            ..Self::default()
        }
    }
}

impl From<TextStyle<Rgba>> for TextStyle<ColorRef> {
    fn from(style: TextStyle<Rgba>) -> Self {
        style
            .map_colors(|c| Ok::<_, std::convert::Infallible>(ColorRef::Rgba(*c)))
            .unwrap_or_else(|never| match never {})
    }
}

/// Font settings of a resolved theme. Families are lists in order of
/// preference; sizes are in points, the line spacing is a multiplier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Fonts {
    pub ui_family: Vec<String>,
    pub ui_size: f32,
    pub editor_family: Vec<String>,
    pub editor_size: f32,
    pub editor_line_spacing: f32,
    pub editor_ligatures: bool,
    pub terminal_family: Vec<String>,
    pub terminal_size: f32,
}

/// The part of [`Fonts`] a theme file sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FontPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_family: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_family: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_line_spacing: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_ligatures: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_family: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_size: Option<f32>,
}

impl FontPatch {
    /// Values set in `over` win.
    pub fn overlay(&self, over: &FontPatch) -> FontPatch {
        FontPatch {
            ui_family: over.ui_family.clone().or_else(|| self.ui_family.clone()),
            ui_size: over.ui_size.or(self.ui_size),
            editor_family: over
                .editor_family
                .clone()
                .or_else(|| self.editor_family.clone()),
            editor_size: over.editor_size.or(self.editor_size),
            editor_line_spacing: over.editor_line_spacing.or(self.editor_line_spacing),
            editor_ligatures: over.editor_ligatures.or(self.editor_ligatures),
            terminal_family: over
                .terminal_family
                .clone()
                .or_else(|| self.terminal_family.clone()),
            terminal_size: over.terminal_size.or(self.terminal_size),
        }
    }

    /// Names of the settings that are not set.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut check = |set: bool, name: &'static str| {
            if !set {
                out.push(name);
            }
        };
        check(self.ui_family.is_some(), "font.ui-family");
        check(self.ui_size.is_some(), "font.ui-size");
        check(self.editor_family.is_some(), "font.editor-family");
        check(self.editor_size.is_some(), "font.editor-size");
        check(
            self.editor_line_spacing.is_some(),
            "font.editor-line-spacing",
        );
        check(self.editor_ligatures.is_some(), "font.editor-ligatures");
        check(self.terminal_family.is_some(), "font.terminal-family");
        check(self.terminal_size.is_some(), "font.terminal-size");
        out
    }

    /// The complete settings, when every value is set.
    pub fn complete(&self) -> Option<Fonts> {
        Some(Fonts {
            ui_family: self.ui_family.clone()?,
            ui_size: self.ui_size?,
            editor_family: self.editor_family.clone()?,
            editor_size: self.editor_size?,
            editor_line_spacing: self.editor_line_spacing?,
            editor_ligatures: self.editor_ligatures?,
            terminal_family: self.terminal_family.clone()?,
            terminal_size: self.terminal_size?,
        })
    }
}

impl From<&Fonts> for FontPatch {
    fn from(f: &Fonts) -> Self {
        Self {
            ui_family: Some(f.ui_family.clone()),
            ui_size: Some(f.ui_size),
            editor_family: Some(f.editor_family.clone()),
            editor_size: Some(f.editor_size),
            editor_line_spacing: Some(f.editor_line_spacing),
            editor_ligatures: Some(f.editor_ligatures),
            terminal_family: Some(f.terminal_family.clone()),
            terminal_size: Some(f.terminal_size),
        }
    }
}

/// Colours of all [`UiRole`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct UiColors([Rgba; UiRole::COUNT]);

impl UiColors {
    pub fn filled(color: Rgba) -> Self {
        Self([color; UiRole::COUNT])
    }

    pub fn iter(&self) -> impl Iterator<Item = (UiRole, Rgba)> + '_ {
        UiRole::ALL.iter().map(|&role| (role, self[role]))
    }
}

impl Index<UiRole> for UiColors {
    type Output = Rgba;
    fn index(&self, role: UiRole) -> &Rgba {
        &self.0[role.index()]
    }
}

impl IndexMut<UiRole> for UiColors {
    fn index_mut(&mut self, role: UiRole) -> &mut Rgba {
        &mut self.0[role.index()]
    }
}

/// Styles of all [`TextRole`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyles([TextStyle; TextRole::COUNT]);

impl Default for TextStyles {
    fn default() -> Self {
        Self(std::array::from_fn(|_| TextStyle::default()))
    }
}

impl TextStyles {
    pub fn iter(&self) -> impl Iterator<Item = (TextRole, &TextStyle)> + '_ {
        TextRole::ALL.iter().map(|&role| (role, &self[role]))
    }
}

impl Index<TextRole> for TextStyles {
    type Output = TextStyle;
    fn index(&self, role: TextRole) -> &TextStyle {
        &self.0[role.index()]
    }
}

impl IndexMut<TextRole> for TextStyles {
    fn index_mut(&mut self, role: TextRole) -> &mut TextStyle {
        &mut self.0[role.index()]
    }
}

/// Layout numbers of all [`Metric`]s in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics([f32; Metric::COUNT]);

impl Metrics {
    pub fn filled(value: f32) -> Self {
        Self([value; Metric::COUNT])
    }

    pub fn iter(&self) -> impl Iterator<Item = (Metric, f32)> + '_ {
        Metric::ALL.iter().map(|&metric| (metric, self[metric]))
    }
}

impl Index<Metric> for Metrics {
    type Output = f32;
    fn index(&self, metric: Metric) -> &f32 {
        &self.0[metric.index()]
    }
}

impl IndexMut<Metric> for Metrics {
    fn index_mut(&mut self, metric: Metric) -> &mut f32 {
        &mut self.0[metric.index()]
    }
}

/// A complete theme: every role has a value.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub kind: ThemeKind,
    pub author: Option<String>,
    pub description: Option<String>,
    pub ui: UiColors,
    pub text: TextStyles,
    pub fonts: Fonts,
    pub metrics: Metrics,
}

impl Theme {
    /// Default text colour of the editor for a role: its own foreground, or
    /// the editor text colour.
    pub fn text_color(&self, role: TextRole) -> Rgba {
        self.text[role].fg.unwrap_or(self.ui[UiRole::EditorText])
    }

    /// Converts to a self-contained theme file (no `extends`) that resolves to
    /// the same theme. Plain text styles are left out.
    pub fn to_file(&self) -> ThemeFile {
        ThemeFile {
            theme: ThemeHeader {
                format: Some(FORMAT_VERSION),
                id: self.id.clone(),
                name: self.name.clone(),
                kind: Some(self.kind),
                author: self.author.clone(),
                description: self.description.clone(),
                extends: None,
            },
            palette: BTreeMap::new(),
            ui: self
                .ui
                .iter()
                .map(|(r, c)| (r, ColorRef::Rgba(c)))
                .collect(),
            syntax: self
                .text
                .iter()
                .filter(|(_, s)| !s.is_plain())
                .map(|(r, s)| (r, s.clone().into()))
                .collect(),
            font: FontPatch::from(&self.fonts),
            metrics: self.metrics.iter().collect(),
        }
    }
}

/// Version of the theme file format written by this crate.
pub const FORMAT_VERSION: u32 = 1;

/// The `[theme]` table of a theme file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ThemeHeader {
    /// Version of the file format; absent means 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    /// Identifier used by `extends` and by the settings.
    pub id: String,
    pub name: String,
    /// Required unless inherited from `extends`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ThemeKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Id of the theme whose values are used for everything this file omits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
}

/// A theme as written in a TOML file: sparse, with optional `extends` and
/// `$palette` references.
///
/// ```toml
/// [theme]
/// id = "my-dark"
/// name = "My Dark"
/// extends = "pyoxide-dark"
///
/// [palette]
/// rust = "#c8643c"
///
/// [ui]
/// accent = "$rust"
///
/// [syntax]
/// keyword = { fg = "$rust", bold = true }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFile {
    pub theme: ThemeHeader,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub palette: BTreeMap<String, Rgba>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub ui: BTreeMap<UiRole, ColorRef>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub syntax: BTreeMap<TextRole, TextStyle<ColorRef>>,
    #[serde(skip_serializing_if = "font_patch_is_empty")]
    pub font: FontPatch,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<Metric, f32>,
}

fn font_patch_is_empty(patch: &FontPatch) -> bool {
    *patch == FontPatch::default()
}

impl ThemeFile {
    /// Reads a theme file; `origin` names it in error messages.
    pub fn from_toml(text: &str, origin: &str) -> Result<Self, crate::ThemeError> {
        let file: ThemeFile = toml::from_str(text).map_err(|e| crate::ThemeError::Parse {
            origin: origin.to_owned(),
            message: e.to_string(),
        })?;
        if let Some(format) = file.theme.format
            && format > FORMAT_VERSION
        {
            return Err(crate::ThemeError::UnsupportedFormat {
                origin: origin.to_owned(),
                format,
            });
        }
        Ok(file)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a theme file is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_keeps_unset_fields() {
        let base = TextStyle {
            fg: Some(Rgba::rgb(1, 2, 3)),
            bold: Some(true),
            ..TextStyle::default()
        };
        let over = TextStyle {
            fg: Some(Rgba::rgb(9, 9, 9)),
            italic: Some(true),
            ..TextStyle::default()
        };
        let merged = base.overlay(&over);
        assert_eq!(merged.fg, Some(Rgba::rgb(9, 9, 9)));
        assert_eq!(merged.bold, Some(true));
        assert_eq!(merged.italic, Some(true));
        assert!(TextStyle::<Rgba>::default().is_plain());
        assert!(!merged.is_plain());
    }

    #[test]
    fn text_style_toml_uses_kebab_case_and_rejects_typos() {
        let style: TextStyle<ColorRef> =
            toml::from_str("fg = \"#112233\"\nunderline = \"wavy\"\nunderline-color = \"$x\"\n")
                .unwrap();
        assert_eq!(style.underline, Some(UnderlineStyle::Wavy));
        assert_eq!(style.underline_color, Some(ColorRef::Palette("x".into())));
        assert!(toml::from_str::<TextStyle<ColorRef>>("fgg = \"#112233\"").is_err());
    }

    #[test]
    fn theme_file_round_trips_through_toml() {
        let mut file = ThemeFile::default();
        file.theme.id = "t".into();
        file.theme.name = "T".into();
        file.theme.kind = Some(ThemeKind::Dark);
        file.theme.extends = Some("pyoxide-dark".into());
        file.palette.insert("red".into(), Rgba::rgb(200, 0, 0));
        file.ui
            .insert(UiRole::Accent, ColorRef::Palette("red".into()));
        file.syntax.insert(
            TextRole::Keyword,
            TextStyle {
                fg: Some(ColorRef::Rgba(Rgba::rgb(1, 2, 3))),
                bold: Some(true),
                ..TextStyle::default()
            },
        );
        file.metrics.insert(Metric::RowHeight, 24.0);
        file.font.editor_size = Some(13.5);
        let text = file.to_toml();
        let back = ThemeFile::from_toml(&text, "test").unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn unknown_role_in_a_theme_file_is_an_error() {
        let err = ThemeFile::from_toml("[ui]\nnot-a-role = \"#000000\"\n", "x.toml").unwrap_err();
        assert!(err.to_string().contains("not-a-role"), "{err}");
    }

    #[test]
    fn newer_format_is_refused() {
        let err = ThemeFile::from_toml("[theme]\nformat = 99\nid = \"a\"\nname = \"a\"\n", "x")
            .unwrap_err();
        assert!(matches!(
            err,
            crate::ThemeError::UnsupportedFormat { format: 99, .. }
        ));
    }
}
