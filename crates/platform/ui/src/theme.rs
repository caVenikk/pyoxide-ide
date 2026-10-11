//! The current theme as a GPUI global: colours of the `themes` crate as
//! GPUI colours, metrics as pixels and the font families that are actually
//! available.

use gpui::{App, Global, Hsla, Pixels, Rgba, SharedString, px};
use themes::{Metric, TextRole, TextStyle, ThemeKind, UiRole};

/// The resolved theme of the application.
pub struct Theme {
    inner: themes::Theme,
    colors: Vec<Hsla>,
    ui_family: SharedString,
    mono_family: SharedString,
}

impl Global for Theme {}

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
        let colors = UiRole::ALL.iter().map(|&r| color(inner.ui[r])).collect();
        let ui_family = crate::fonts::pick_family(&inner.fonts.ui_family, available).into();
        let mono_family = crate::fonts::pick_family(&inner.fonts.editor_family, available).into();
        Self {
            inner,
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

/// Makes `theme` the current theme and redraws every window.
pub fn set(cx: &mut App, theme: themes::Theme) {
    let available = cx.text_system().all_font_names();
    cx.set_global(Theme::new(theme, &available));
    cx.refresh_windows();
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
}
