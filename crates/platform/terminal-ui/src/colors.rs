//! The terminal's colours: the theme's terminal roles (the reference's
//! `BLOCK_TERMINAL_*` keys, see docs/theme-mapping.md) as a palette of
//! text and cell background colours, the colours a program changed, and
//! the resolution of a cell's colours with the reference's rules for
//! inverse and dim text and its minimum contrast
//! (`TerminalUiUtils.toTextAttributes`, IntelliJ Community, Apache-2.0).

use std::collections::HashMap;

use terminal::{CellFlags, Color, Rgb};
use themes::contrast::{TERMINAL_MIN_CONTRAST, ensure_contrast};
use themes::{Rgba, UiRole};

/// ANSI colours 0 to 15: text roles, then cell background roles.
const ANSI_TEXT: [UiRole; 16] = [
    UiRole::TerminalAnsiBlack,
    UiRole::TerminalAnsiRed,
    UiRole::TerminalAnsiGreen,
    UiRole::TerminalAnsiYellow,
    UiRole::TerminalAnsiBlue,
    UiRole::TerminalAnsiMagenta,
    UiRole::TerminalAnsiCyan,
    UiRole::TerminalAnsiWhite,
    UiRole::TerminalAnsiBrightBlack,
    UiRole::TerminalAnsiBrightRed,
    UiRole::TerminalAnsiBrightGreen,
    UiRole::TerminalAnsiBrightYellow,
    UiRole::TerminalAnsiBrightBlue,
    UiRole::TerminalAnsiBrightMagenta,
    UiRole::TerminalAnsiBrightCyan,
    UiRole::TerminalAnsiBrightWhite,
];
const ANSI_BACKGROUND: [UiRole; 16] = [
    UiRole::TerminalAnsiBlackBackground,
    UiRole::TerminalAnsiRedBackground,
    UiRole::TerminalAnsiGreenBackground,
    UiRole::TerminalAnsiYellowBackground,
    UiRole::TerminalAnsiBlueBackground,
    UiRole::TerminalAnsiMagentaBackground,
    UiRole::TerminalAnsiCyanBackground,
    UiRole::TerminalAnsiWhiteBackground,
    UiRole::TerminalAnsiBrightBlackBackground,
    UiRole::TerminalAnsiBrightRedBackground,
    UiRole::TerminalAnsiBrightGreenBackground,
    UiRole::TerminalAnsiBrightYellowBackground,
    UiRole::TerminalAnsiBrightBlueBackground,
    UiRole::TerminalAnsiBrightMagentaBackground,
    UiRole::TerminalAnsiBrightCyanBackground,
    UiRole::TerminalAnsiBrightWhiteBackground,
];

/// The colour of palette entry 16 to 255: the 6x6x6 cube and the grey
/// ramp of xterm.
pub fn cube_color(index: u8) -> Rgba {
    match index {
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgba::rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            Rgba::rgb(v, v, v)
        }
        _ => Rgba::BLACK,
    }
}

/// The palette a view paints with.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalColors {
    pub foreground: Rgba,
    pub background: Rgba,
    /// Text colours of palette entries 0 to 255.
    pub text: Vec<Rgba>,
    /// Cell backgrounds of palette entries 0 to 255.
    pub cells: Vec<Rgba>,
    /// The cursor colour a program set (OSC 12); otherwise the cursor
    /// takes the colour of the text under it.
    pub cursor: Option<Rgba>,
    pub selection: Rgba,
    pub search_match: Rgba,
    pub search_match_text: Rgba,
    pub current_match: Rgba,
    pub current_match_text: Rgba,
    /// The minimum contrast of text with its background.
    pub min_contrast: f64,
}

impl TerminalColors {
    pub fn from_theme(theme: &themes::Theme) -> Self {
        let ui = |role: UiRole| theme.ui[role];
        let background = opaque(ui(UiRole::TerminalDefaultBackground), Rgba::BLACK);
        let on_bg = |role: UiRole| opaque(ui(role), background);
        let mut text: Vec<Rgba> = (0..=255u8).map(cube_color).collect();
        let mut cells = text.clone();
        for i in 0..16 {
            text[i] = on_bg(ANSI_TEXT[i]);
            cells[i] = on_bg(ANSI_BACKGROUND[i]);
        }
        Self {
            foreground: on_bg(UiRole::TerminalDefaultForeground),
            background,
            text,
            cells,
            cursor: None,
            selection: ui(UiRole::EditorSelection),
            search_match: on_bg(UiRole::TerminalSearchMatch),
            search_match_text: ui(UiRole::TerminalSearchMatchText),
            current_match: on_bg(UiRole::TerminalSearchMatchCurrent),
            current_match_text: ui(UiRole::TerminalSearchMatchCurrentText),
            min_contrast: TERMINAL_MIN_CONTRAST,
        }
    }

    /// These colours with the ones a program set (OSC 4, 10, 11, 12): a
    /// palette entry changes both its text and its cell colour.
    pub fn with_overrides(&self, overrides: &[(Color, Rgb)]) -> Self {
        let mut out = self.clone();
        for &(color, rgb) in overrides {
            let c = Rgba::rgb(rgb.r, rgb.g, rgb.b);
            match color {
                Color::Indexed(i) => {
                    out.text[i as usize] = c;
                    out.cells[i as usize] = c;
                }
                Color::Foreground => out.foreground = c,
                Color::Background => out.background = c,
                Color::Cursor => out.cursor = Some(c),
                _ => {}
            }
        }
        out
    }

    /// The colours the session answers colour queries with.
    pub fn palette(&self) -> terminal::Palette {
        let rgb = |c: Rgba| Rgb::new(c.r, c.g, c.b);
        let mut ansi = [Rgb::new(0, 0, 0); 16];
        for (i, slot) in ansi.iter_mut().enumerate() {
            *slot = rgb(self.text[i]);
        }
        terminal::Palette {
            ansi,
            foreground: rgb(self.foreground),
            background: rgb(self.background),
            cursor: rgb(self.cursor.unwrap_or(self.foreground)),
        }
    }

    /// A text colour: the colour, and whether a program gave it as RGB.
    fn text_color(&self, color: Color) -> (Rgba, bool) {
        match color {
            Color::Indexed(i) => (self.text[i as usize], false),
            Color::Rgb(c) => (Rgba::rgb(c.r, c.g, c.b), true),
            Color::Background => (self.background, false),
            Color::Cursor => (self.cursor.unwrap_or(self.foreground), false),
            Color::Dim(i) => (self.text[i.min(15) as usize], false),
            Color::Foreground | Color::BrightForeground | Color::DimForeground => {
                (self.foreground, false)
            }
        }
    }

    /// A cell background: `None` for the default background (not painted
    /// per cell), and whether a program gave it as RGB.
    fn cell_color(&self, color: Color) -> (Option<Rgba>, bool) {
        match color {
            Color::Background => (None, false),
            Color::Indexed(i) => (Some(self.cells[i as usize]), false),
            Color::Rgb(c) => (Some(Rgba::rgb(c.r, c.g, c.b)), true),
            Color::Dim(i) => (Some(self.cells[i.min(15) as usize]), false),
            Color::Foreground | Color::BrightForeground | Color::DimForeground => {
                (Some(self.foreground), false)
            }
            Color::Cursor => (Some(self.cursor.unwrap_or(self.foreground)), false),
        }
    }
}

/// A colour composited over `backdrop` when it is translucent.
fn opaque(c: Rgba, backdrop: Rgba) -> Rgba {
    if c.is_opaque() { c } else { c.over(backdrop) }
}

/// The colours a cell is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellLook {
    pub text: Rgba,
    /// `None`: the default background.
    pub background: Option<Rgba>,
}

/// Characters whose colour is never adjusted for contrast: Powerline
/// symbols and box and block drawing (`shouldIgnoreContrastAdjustment`).
pub fn keeps_its_colour(c: char) -> bool {
    matches!(c as u32, 0xE0A4..=0xE0D6 | 0x2500..=0x259F)
}

/// Resolves cells' colours, remembering the contrast adjustments (a
/// screen has few distinct colour pairs).
#[derive(Default)]
pub struct Resolver {
    adjusted: HashMap<(Rgba, Rgba, u16), Rgba>,
}

impl Resolver {
    /// The colours of a cell with its text attributes: inverse swaps the
    /// text colour (taken from the cell background palette) and the
    /// background (from the text palette); dim text is half way to the
    /// background; the text keeps the minimum contrast with its background
    /// unless both colours are RGB, at half the ratio when dim.
    pub fn look(
        &mut self,
        colors: &TerminalColors,
        fg: Color,
        bg: Color,
        flags: CellFlags,
        c: char,
    ) -> CellLook {
        let inverse = flags.contains(CellFlags::INVERSE);
        let (text, text_rgb, background, background_rgb) = if inverse {
            let (cell, cell_rgb) = colors.cell_color(bg);
            let (text, text_rgb) = colors.text_color(fg);
            (
                cell.unwrap_or(colors.background),
                cell_rgb,
                Some(text),
                text_rgb,
            )
        } else {
            let (text, text_rgb) = colors.text_color(fg);
            let (cell, cell_rgb) = colors.cell_color(bg);
            (text, text_rgb, cell, cell_rgb)
        };
        let behind = background.unwrap_or(colors.background);
        let dim = flags.contains(CellFlags::DIM);
        let text = if dim { mix(text, behind) } else { text };
        let ratio = if text_rgb && background_rgb || keeps_its_colour(c) {
            1.0
        } else if dim {
            colors.min_contrast / 2.0
        } else {
            colors.min_contrast
        };
        CellLook {
            text: self.contrasted(text, behind, ratio),
            background,
        }
    }

    /// `text` with at least `ratio` contrast against `behind`.
    pub fn contrasted(&mut self, text: Rgba, behind: Rgba, ratio: f64) -> Rgba {
        if ratio <= 1.0 {
            return text;
        }
        let key = (text, behind, (ratio * 100.0) as u16);
        *self
            .adjusted
            .entry(key)
            .or_insert_with(|| ensure_contrast(text, behind, ratio))
    }
}

/// The colour half way between two colours.
fn mix(a: Rgba, b: Rgba) -> Rgba {
    let m = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
    Rgba::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use themes::contrast_ratio;

    fn colors() -> TerminalColors {
        TerminalColors::from_theme(&themes::builtin::default_theme(themes::ThemeKind::Dark))
    }

    #[test]
    fn theme_roles_fill_the_palette() {
        let theme = themes::builtin::default_theme(themes::ThemeKind::Dark);
        let c = colors();
        assert_eq!(c.text[1], theme.ui[UiRole::TerminalAnsiRed]);
        assert_eq!(c.cells[1], theme.ui[UiRole::TerminalAnsiRedBackground]);
        assert_ne!(c.text[1], c.cells[1], "text and cell colours differ");
        assert_eq!(c.text[196], Rgba::rgb(255, 0, 0));
        assert_eq!(c.text[232], Rgba::rgb(8, 8, 8));
        assert_eq!(c.background, theme.ui[UiRole::TerminalDefaultBackground]);
        let p = c.palette();
        assert_eq!(p.ansi[1], Rgb::new(c.text[1].r, c.text[1].g, c.text[1].b));
    }

    #[test]
    fn overrides_change_text_and_cells() {
        let c = colors().with_overrides(&[
            (Color::Indexed(1), Rgb::new(1, 2, 3)),
            (Color::Background, Rgb::new(9, 9, 9)),
            (Color::Cursor, Rgb::new(7, 7, 7)),
        ]);
        assert_eq!(c.text[1], Rgba::rgb(1, 2, 3));
        assert_eq!(c.cells[1], Rgba::rgb(1, 2, 3));
        assert_eq!(c.background, Rgba::rgb(9, 9, 9));
        assert_eq!(c.cursor, Some(Rgba::rgb(7, 7, 7)));
    }

    #[test]
    fn inverse_dim_and_contrast() {
        let c = colors();
        let mut r = Resolver::default();
        // Plain red: the text palette, default background, readable.
        let red = r.look(
            &c,
            Color::Indexed(1),
            Color::Background,
            CellFlags::empty(),
            'x',
        );
        assert_eq!(red.background, None);
        assert!(contrast_ratio(red.text, c.background, c.background) >= 4.5);
        // Inverse: the cell palette's red as text colour source swaps in.
        let inv = r.look(
            &c,
            Color::Indexed(1),
            Color::Background,
            CellFlags::INVERSE,
            'x',
        );
        assert_eq!(inv.background, Some(c.text[1]));
        // Black text on the default dark background is lifted to 4.5.
        let black = r.look(
            &c,
            Color::Indexed(0),
            Color::Background,
            CellFlags::empty(),
            'x',
        );
        assert_ne!(black.text, c.text[0]);
        assert!(contrast_ratio(black.text, c.background, c.background) >= 4.5);
        // ... but not a box drawing character, and not RGB on RGB.
        let boxed = r.look(
            &c,
            Color::Indexed(0),
            Color::Background,
            CellFlags::empty(),
            '─',
        );
        assert_eq!(boxed.text, c.text[0]);
        let rgb = r.look(
            &c,
            Color::Rgb(Rgb::new(10, 10, 10)),
            Color::Rgb(Rgb::new(12, 12, 12)),
            CellFlags::empty(),
            'x',
        );
        assert_eq!(rgb.text, Rgba::rgb(10, 10, 10));
        // Dim: half way to the background, half the contrast.
        let dim = r.look(
            &c,
            Color::Foreground,
            Color::Background,
            CellFlags::DIM,
            'x',
        );
        assert_ne!(dim.text, c.foreground);
        assert!(contrast_ratio(dim.text, c.background, c.background) >= 2.25);
    }
}
