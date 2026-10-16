//! The current theme as a GPUI global: colours of the `themes` crate as
//! GPUI colours, metrics as pixels and the font families that are actually
//! available, with the editor font chosen in the settings
//! ([`FontChoice`]) over the theme's.

use gpui::{
    App, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Global, Hsla, Pixels, Rgba,
    SharedString, px,
};
use themes::{Metric, TextRole, TextStyle, ThemeKind, UiRole};

/// The resolved theme of the application.
pub struct Theme {
    /// The theme with the editor font chosen over its own.
    inner: themes::Theme,
    /// The theme as given.
    base: themes::Theme,
    choice: FontChoice,
    /// The font families the text system knew when the theme was set.
    available: Vec<String>,
    colors: Vec<Hsla>,
    ui_family: SharedString,
    mono_family: SharedString,
}

impl Global for Theme {}

/// The editor font chosen in the settings (Editor > Font) over the theme's;
/// what is not chosen stays the theme's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FontChoice {
    pub family: Option<String>,
    /// In points.
    pub size: Option<f32>,
    /// A multiple of the font's own line height.
    pub line_spacing: Option<f32>,
    pub ligatures: Option<bool>,
    /// The family of the characters the font lacks.
    pub fallback: Option<String>,
}

/// The editor font in use: family, size, line spacing, ligatures and the
/// fallback family.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorFont {
    pub family: SharedString,
    pub size: Pixels,
    pub line_spacing: f32,
    pub ligatures: bool,
    pub fallback: Option<SharedString>,
}

impl EditorFont {
    /// The regular GPUI font: ligatures off unless chosen, the fallback
    /// family first for the characters the font lacks.
    pub fn font(&self) -> Font {
        Font {
            family: self.family.clone(),
            features: if self.ligatures {
                FontFeatures::default()
            } else {
                FontFeatures::disable_ligatures()
            },
            fallbacks: self
                .fallback
                .as_ref()
                .map(|f| FontFallbacks::from_fonts(vec![f.to_string()])),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }
}

/// The editor font chosen in the settings, kept across theme changes.
#[derive(Default)]
struct Chosen(FontChoice);

impl Global for Chosen {}

/// The theme's fonts with the editor font of `choice` over its own: the
/// chosen family first in the list of families (the next one is used when
/// the text system does not know it).
fn chosen_fonts(base: &themes::Fonts, choice: &FontChoice) -> themes::Fonts {
    let mut fonts = base.clone();
    if let Some(family) = &choice.family {
        let rest = base.editor_family.iter().filter(|f| *f != family);
        fonts.editor_family = std::iter::once(family).chain(rest).cloned().collect();
    }
    if let Some(size) = choice.size {
        fonts.editor_size = size;
    }
    if let Some(spacing) = choice.line_spacing {
        fonts.editor_line_spacing = spacing;
    }
    if let Some(ligatures) = choice.ligatures {
        fonts.editor_ligatures = ligatures;
    }
    fonts
}

/// GPUI colour of a theme colour.
pub fn color(c: themes::Rgba) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: f32::from(c.a) / 255.0,
    }
    .into()
}

impl Theme {
    /// Wraps a resolved theme; `available` lists the font families the text
    /// system knows (the first available family of each list is used).
    pub fn new(inner: themes::Theme, available: &[String]) -> Self {
        Self::with_choice(inner, available, FontChoice::default())
    }

    /// The theme with the editor font of `choice` over its own; a chosen
    /// family the text system does not know gives the theme's.
    pub fn with_choice(base: themes::Theme, available: &[String], choice: FontChoice) -> Self {
        let mut inner = base.clone();
        inner.fonts = chosen_fonts(&base.fonts, &choice);
        let colors = UiRole::ALL.iter().map(|&r| color(inner.ui[r])).collect();
        let ui_family = crate::fonts::pick_family(&inner.fonts.ui_family, available).into();
        let mono_family = crate::fonts::pick_family(&inner.fonts.editor_family, available).into();
        Self {
            inner,
            base,
            choice,
            available: available.to_vec(),
            colors,
            ui_family,
            mono_family,
        }
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn kind(&self) -> ThemeKind {
        self.inner.kind
    }

    pub fn is_dark(&self) -> bool {
        self.inner.kind == ThemeKind::Dark
    }

    /// Colour of a UI role.
    pub fn color(&self, role: UiRole) -> Hsla {
        self.colors[role.index()]
    }

    /// A metric in logical pixels.
    pub fn metric(&self, metric: Metric) -> Pixels {
        px(self.inner.metrics[metric])
    }

    /// Text attributes of a text role, as the theme defines them.
    pub fn text_style(&self, role: TextRole) -> &TextStyle {
        &self.inner.text[role]
    }

    /// Foreground of a text role, or the editor text colour.
    pub fn text_color(&self, role: TextRole) -> Hsla {
        color(self.inner.text_color(role))
    }

    pub fn ui_family(&self) -> SharedString {
        self.ui_family.clone()
    }

    pub fn ui_size(&self) -> Pixels {
        px(self.inner.fonts.ui_size)
    }

    pub fn mono_family(&self) -> SharedString {
        self.mono_family.clone()
    }

    pub fn editor_size(&self) -> Pixels {
        px(self.inner.fonts.editor_size)
    }

    /// Multiplier of the editor font's natural line height.
    pub fn editor_line_spacing(&self) -> f32 {
        self.inner.fonts.editor_line_spacing
    }

    /// The editor font in use.
    pub fn editor_font(&self) -> EditorFont {
        EditorFont {
            family: self.mono_family.clone(),
            size: self.editor_size(),
            line_spacing: self.editor_line_spacing(),
            ligatures: self.inner.fonts.editor_ligatures,
            fallback: self.choice.fallback.clone().map(Into::into),
        }
    }

    /// The editor font `choice` would give (the preview of Editor > Font).
    pub fn editor_font_with(&self, choice: &FontChoice) -> EditorFont {
        let fonts = chosen_fonts(&self.base.fonts, choice);
        EditorFont {
            family: crate::fonts::pick_family(&fonts.editor_family, &self.available).into(),
            size: px(fonts.editor_size),
            line_spacing: fonts.editor_line_spacing,
            ligatures: fonts.editor_ligatures,
            fallback: choice.fallback.clone().map(Into::into),
        }
    }

    /// The theme's own editor font, without the one chosen in the
    /// settings.
    pub fn theme_editor_font(&self) -> EditorFont {
        self.editor_font_with(&FontChoice::default())
    }

    /// The editor font chosen in the settings.
    pub fn font_choice(&self) -> &FontChoice {
        &self.choice
    }

    /// The `themes` crate's theme.
    pub fn raw(&self) -> &themes::Theme {
        &self.inner
    }
}

/// Shorthand for `cx.global::<Theme>()`.
pub trait ThemeExt {
    fn theme(&self) -> &Theme;
}

impl ThemeExt for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

/// Installs the default theme of `kind`.
pub fn init(cx: &mut App, kind: ThemeKind) {
    set(cx, themes::builtin::default_theme(kind));
}

/// Makes `theme` the current theme, with the editor font chosen in the
/// settings over its own, and redraws every window.
pub fn set(cx: &mut App, theme: themes::Theme) {
    let available = cx.text_system().all_font_names();
    let choice = cx
        .try_global::<Chosen>()
        .map(|c| c.0.clone())
        .unwrap_or_default();
    cx.set_global(Theme::with_choice(theme, &available, choice));
    cx.refresh_windows();
}

/// Puts the editor font chosen in the settings over the theme's (now and
/// for the themes set later) and redraws every window: editors lay their
/// text out again. The font families known when the theme was set are
/// used; nothing is enumerated.
pub fn set_font_choice(cx: &mut App, choice: FontChoice) {
    if cx.try_global::<Chosen>().is_some_and(|c| c.0 == choice) {
        return;
    }
    cx.set_global(Chosen(choice.clone()));
    if let Some(theme) = cx.try_global::<Theme>() {
        let theme = Theme::with_choice(theme.base.clone(), &theme.available, choice);
        cx.set_global(theme);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_colours_and_metrics() {
        let theme = Theme::new(themes::builtin::dark(), &["JetBrains Mono".to_owned()]);
        assert!(theme.is_dark());
        let c = color(themes::Rgba::new(255, 0, 0, 255));
        assert_eq!(
            Rgba::from(c),
            Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            }
        );
        assert_eq!(theme.metric(Metric::ToolbarHeight), px(42.5));
        assert_eq!(theme.mono_family().as_ref(), "JetBrains Mono");
        // Inter is not in the list: the first family is used as is.
        assert_eq!(theme.ui_family().as_ref(), "Inter");
        assert_eq!(theme.editor_line_spacing(), 1.2);
    }

    #[test]
    fn the_chosen_editor_font_goes_over_the_theme() {
        let available = ["JetBrains Mono".to_owned(), "Menlo".to_owned()];
        let theme = Theme::new(themes::builtin::dark(), &available);
        let own = theme.editor_font();
        assert_eq!(own.family.as_ref(), "JetBrains Mono");
        assert!(!own.ligatures);
        let choice = FontChoice {
            family: Some("Menlo".into()),
            size: Some(15.0),
            line_spacing: Some(1.5),
            ligatures: Some(true),
            fallback: Some("Symbols".into()),
        };
        let chosen = Theme::with_choice(themes::builtin::dark(), &available, choice.clone());
        let font = chosen.editor_font();
        assert_eq!(font.family.as_ref(), "Menlo");
        assert_eq!(font.size, px(15.0));
        assert_eq!(font.line_spacing, 1.5);
        assert!(font.ligatures && chosen.raw().fonts.editor_ligatures);
        assert_eq!(chosen.mono_family().as_ref(), "Menlo");
        let fallbacks = font.font().fallbacks.expect("a fallback family");
        assert_eq!(fallbacks.fallback_list(), ["Symbols".to_owned()]);
        assert_eq!(chosen.theme_editor_font(), own);
        assert_eq!(theme.editor_font_with(&choice), font);
        // A family the text system does not know: the theme's.
        let missing = FontChoice {
            family: Some("Nope".into()),
            ..FontChoice::default()
        };
        let theme = Theme::with_choice(themes::builtin::dark(), &available, missing);
        assert_eq!(theme.mono_family().as_ref(), "JetBrains Mono");
    }
}
