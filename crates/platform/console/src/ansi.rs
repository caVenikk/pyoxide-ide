/// A colour set by an SGR sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// One of the 16 theme colours (0-7 normal, 8-15 bright: black, red,
    /// green, yellow, blue, magenta, cyan, white). A view takes them from the
    /// theme's console ANSI colours.
    Ansi(u8),
    /// A colour of the 256-colour palette above 15 (`38;5;n` with n in
    /// 16..=255): the 6x6x6 cube and the grey ramp, see
    /// [`Color::indexed_rgb`].
    Indexed(u8),
    /// A 24-bit colour (`38;2;r;g;b`).
    Rgb(u8, u8, u8),
}

impl Color {
    /// The RGB value of a 256-colour palette index from 16 to 255, as the
    /// reference computes it (`EightBitRGBColor`); `None` below 16 (those
    /// are theme colours).
    pub fn indexed_rgb(index: u8) -> Option<(u8, u8, u8)> {
        const CUBE: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
        match index {
            0..=15 => None,
            16..=231 => {
                let i = (index - 16) as usize;
                Some((CUBE[(i / 36) % 6], CUBE[(i / 6) % 6], CUBE[i % 6]))
            }
            232..=255 => {
                let v = 0x08 + (index - 232) * 10;
                Some((v, v, v))
            }
        }
    }
}

/// How text is underlined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    /// No underline.
    #[default]
    None,
    /// SGR 4.
    Single,
    /// SGR 21. The reference draws it as one thick line.
    Double,
}

/// The SGR attributes of a piece of text.
///
/// `Style::default()` is plain text, which a view colours by its
/// [`ContentKind`](crate::ContentKind). A style with `ansi` set is drawn the
/// reference's way for ANSI text, whatever the stream: `fg` (or the normal
/// output colour when `None`, also on standard error) on `bg` (or the
/// console background), swapped when `inverse`; underline and strike-through
/// in the foreground colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    /// Foreground colour; `None` is the default.
    pub fg: Option<Color>,
    /// Background colour; `None` is the default.
    pub bg: Option<Color>,
    /// SGR 1.
    pub bold: bool,
    /// SGR 2 (faint). Parsed but not drawn by the reference.
    pub dim: bool,
    /// SGR 3.
    pub italic: bool,
    /// SGR 4 and 21.
    pub underline: Underline,
    /// SGR 7: foreground and background swapped.
    pub inverse: bool,
    /// SGR 9.
    pub strikethrough: bool,
    /// Any SGR attribute is active, including ones that are not drawn
    /// (blink, conceal, fonts, frames): the reference then switches from the
    /// stream's colours to ANSI drawing (see the type's documentation).
    pub ansi: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_palette() {
        assert_eq!(Color::indexed_rgb(15), None);
        assert_eq!(Color::indexed_rgb(16), Some((0, 0, 0)));
        assert_eq!(Color::indexed_rgb(196), Some((0xff, 0, 0)));
        assert_eq!(Color::indexed_rgb(231), Some((0xff, 0xff, 0xff)));
        assert_eq!(Color::indexed_rgb(232), Some((0x08, 0x08, 0x08)));
        assert_eq!(Color::indexed_rgb(255), Some((0xee, 0xee, 0xee)));
    }
}
