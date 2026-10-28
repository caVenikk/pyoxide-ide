use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::color::{ColorRef, Rgba};
use crate::roles::{Metric, TextRole, UiRole};

/// Whether a theme is meant for a dark or a light background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Dark,
    Light,
}

/// The look of the controls whose drawing differs between the reference's
/// New UI themes and the classic ones (themes not based on a New UI theme,
/// such as most theme plugins): the size of check boxes, the height of
/// dialog buttons and the shape of tree arrows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Controls {
    /// Check boxes of 16, buttons 28 high, chevrons.
    #[default]
    New,
    /// Check boxes of 14, buttons 24 high, filled triangles.
    Classic,
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
#[serde(default, rename_all = "kebab-case")]
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
    pub controls: Controls,
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
                base: None,
                scheme_only: false,
                controls: (self.controls != Controls::New).then_some(self.controls),
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
///
/// A field this build does not know is skipped when a file is read (see
/// [`ThemeFile`]); a bad value of a known field is an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
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
    /// Id of the built-in theme that supplies the roles a self-contained
    /// file lacks (roles added by later versions): the theme an import was
    /// made on. When absent, the default theme of the file's kind (the
    /// base theme of an import for a file with classic controls) is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The file holds an editor colour scheme without a UI theme (an
    /// imported `.icls`): the IDE offers it as an editor colour scheme
    /// only, and its UI colours are those of its base theme.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scheme_only: bool,
    /// The look of the controls; inherited from `extends`, `new` when no
    /// file of the chain sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
}

/// A theme as written in a TOML file: sparse, with optional `extends` and
/// `$palette` references.
///
/// ```toml
/// [theme]
/// id = "my-dark"
/// name = "My Dark"
/// extends = "app-dark"
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
///
/// A key of `[ui]`, `[syntax]` or `[metrics]` that names no role of this
/// build is skipped, and so is an attribute of a text style or a field of
/// `[theme]` that this build does not know, so a file saved by a newer build
/// that knows more still loads; [`ThemeFile::parse_toml`] returns the skipped
/// keys. Anything else that does not parse (a bad value of a known key) is an
/// error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawThemeFile")]
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

/// A theme role looked up by the name used in theme files.
trait RoleKey: Ord + Sized {
    fn from_name(name: &str) -> Option<Self>;
}

macro_rules! role_key {
    ($($role:ident),*) => {$(
        impl RoleKey for $role {
            fn from_name(name: &str) -> Option<Self> {
                $role::from_name(name)
            }
        }
    )*};
}

role_key!(UiRole, TextRole, Metric);

/// A value of a role table as read: the value and the names of the keys
/// inside it that this build does not know.
trait RoleValue {
    type Value;
    fn into_parts(self) -> (Self::Value, Vec<String>);
}

macro_rules! plain_role_value {
    ($($ty:ty),*) => {$(
        impl RoleValue for $ty {
            type Value = $ty;
            fn into_parts(self) -> (Self::Value, Vec<String>) {
                (self, Vec::new())
            }
        }
    )*};
}

plain_role_value!(ColorRef, f32);

/// A text style as read: attributes this build does not know are kept apart.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawStyle {
    #[serde(flatten)]
    style: TextStyle<ColorRef>,
    #[serde(flatten)]
    unknown: BTreeMap<String, IgnoredAny>,
}

impl RoleValue for RawStyle {
    type Value = TextStyle<ColorRef>;
    fn into_parts(self) -> (Self::Value, Vec<String>) {
        (self.style, self.unknown.into_keys().collect())
    }
}

/// A table of a theme file keyed by role names: the entries of the roles this
/// build knows and the keys it does not (their values are not looked at),
/// with the unknown keys inside the entries as `role.key`.
struct RoleTable<K, V: RoleValue> {
    entries: BTreeMap<K, V::Value>,
    skipped: Vec<String>,
}

impl<K, V: RoleValue> Default for RoleTable<K, V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            skipped: Vec::new(),
        }
    }
}

impl<'de, K: RoleKey, V: RoleValue + Deserialize<'de>> Deserialize<'de> for RoleTable<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RoleVisitor<K, V>(PhantomData<(K, V)>);

        impl<'de, K: RoleKey, V: RoleValue + Deserialize<'de>> Visitor<'de> for RoleVisitor<K, V> {
            type Value = RoleTable<K, V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a table of roles")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut table = RoleTable::default();
                while let Some(key) = map.next_key::<String>()? {
                    match K::from_name(&key) {
                        Some(role) => {
                            let (value, unknown) = map.next_value::<V>()?.into_parts();
                            table.entries.insert(role, value);
                            table
                                .skipped
                                .extend(unknown.into_iter().map(|name| format!("{key}.{name}")));
                        }
                        None => {
                            map.next_value::<IgnoredAny>()?;
                            table.skipped.push(key);
                        }
                    }
                }
                Ok(table)
            }
        }

        deserializer.deserialize_map(RoleVisitor(PhantomData))
    }
}

/// The `[theme]` table as read: fields this build does not know are kept
/// apart.
#[derive(Default, Deserialize)]
struct RawHeader {
    #[serde(flatten)]
    header: ThemeHeader,
    #[serde(flatten)]
    unknown: BTreeMap<String, IgnoredAny>,
}

/// A [`ThemeFile`] as read: the role tables still list the keys that name no
/// role.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawThemeFile {
    theme: RawHeader,
    palette: BTreeMap<String, Rgba>,
    ui: RoleTable<UiRole, ColorRef>,
    syntax: RoleTable<TextRole, RawStyle>,
    font: FontPatch,
    metrics: RoleTable<Metric, f32>,
}

impl RawThemeFile {
    /// The file and the skipped keys, each as `table.key`.
    fn split(self) -> (ThemeFile, Vec<String>) {
        let mut skipped = Vec::new();
        let mut note = |table: &str, keys: Vec<String>| {
            skipped.extend(keys.into_iter().map(|key| format!("{table}.{key}")));
        };
        note("theme", self.theme.unknown.into_keys().collect());
        note("ui", self.ui.skipped);
        note("syntax", self.syntax.skipped);
        note("metrics", self.metrics.skipped);
        let file = ThemeFile {
            theme: self.theme.header,
            palette: self.palette,
            ui: self.ui.entries,
            syntax: self.syntax.entries,
            font: self.font,
            metrics: self.metrics.entries,
        };
        (file, skipped)
    }
}

impl From<RawThemeFile> for ThemeFile {
    fn from(raw: RawThemeFile) -> Self {
        raw.split().0
    }
}

impl ThemeFile {
    /// Reads a theme file; `origin` names it in error messages. The keys
    /// that were skipped (see [`ThemeFile`]) are written to the log in one
    /// warning.
    pub fn from_toml(text: &str, origin: &str) -> Result<Self, crate::ThemeError> {
        let (file, skipped) = Self::parse_toml(text, origin)?;
        if !skipped.is_empty() {
            eprintln!("themes: {}", skipped_warning(origin, &skipped));
        }
        Ok(file)
    }

    /// Reads a theme file and returns, apart from it, the keys that were
    /// skipped because this build does not know them (roles of `[ui]`,
    /// `[syntax]` and `[metrics]`, attributes of a text style, fields of
    /// `[theme]`), each as `table.key` (`syntax.role.attribute` for an
    /// attribute); [`skipped_warning`] words them for the log.
    pub fn parse_toml(text: &str, origin: &str) -> Result<(Self, Vec<String>), crate::ThemeError> {
        let raw: RawThemeFile = toml::from_str(text).map_err(|e| crate::ThemeError::Parse {
            origin: origin.to_owned(),
            message: e.to_string(),
        })?;
        let (file, skipped) = raw.split();
        if let Some(format) = file.theme.format
            && format > FORMAT_VERSION
        {
            return Err(crate::ThemeError::UnsupportedFormat {
                origin: origin.to_owned(),
                format,
            });
        }
        Ok((file, skipped))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a theme file is always serializable")
    }
}

/// The one log line about the keys [`ThemeFile::parse_toml`] skipped in the
/// file `origin`.
pub fn skipped_warning(origin: &str, skipped: &[String]) -> String {
    format!(
        "{origin}: skipped {} key{} this version does not know (saved by a newer one?): {}",
        skipped.len(),
        if skipped.len() == 1 { "" } else { "s" },
        skipped.join(", ")
    )
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
    fn text_style_toml_uses_kebab_case() {
        let style: TextStyle<ColorRef> =
            toml::from_str("fg = \"#112233\"\nunderline = \"wavy\"\nunderline-color = \"$x\"\n")
                .unwrap();
        assert_eq!(style.underline, Some(UnderlineStyle::Wavy));
        assert_eq!(style.underline_color, Some(ColorRef::Palette("x".into())));
    }

    #[test]
    fn theme_file_round_trips_through_toml() {
        let mut file = ThemeFile::default();
        file.theme.id = "t".into();
        file.theme.name = "T".into();
        file.theme.kind = Some(ThemeKind::Dark);
        file.theme.extends = Some("app-dark".into());
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
    fn unknown_roles_are_skipped_and_listed() {
        let text = "[ui]\nnot-a-role = \"#000000\"\naccent = \"#112233\"\n\
                    [syntax]\nnot-a-token = { fg = \"#000000\", future-attribute = 1 }\n\
                    [metrics]\nnot-a-metric = 3.0\n";
        let (file, skipped) = ThemeFile::parse_toml(text, "x.toml").unwrap();
        assert_eq!(
            skipped,
            [
                "ui.not-a-role",
                "syntax.not-a-token",
                "metrics.not-a-metric"
            ]
        );
        assert_eq!(file.ui.len(), 1);
        assert_eq!(
            file.ui[&UiRole::Accent],
            ColorRef::Rgba(Rgba::rgb(0x11, 0x22, 0x33))
        );
        assert!(file.syntax.is_empty() && file.metrics.is_empty());
    }

    #[test]
    fn a_bad_value_of_a_known_role_is_still_an_error() {
        let err = ThemeFile::from_toml("[ui]\naccent = \"nonsense\"\n", "x.toml").unwrap_err();
        assert!(err.to_string().contains("nonsense"), "{err}");
        assert!(ThemeFile::from_toml("[ui]\naccent = \n", "x.toml").is_err());
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
