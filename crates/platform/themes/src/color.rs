use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with straight (non-premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Why a colour string could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a colour: {0:?}")]
pub struct ColorParseError(pub String);

impl Rgba {
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// Parses `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn parse(text: &str) -> Result<Self, ColorParseError> {
        let err = || ColorParseError(text.to_owned());
        let hex = text.trim().trim_start_matches('#');
        if !hex.is_ascii() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map(|v| v * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
        let parsed = match hex.len() {
            3 => (nibble(0), nibble(1), nibble(2), Ok(255)),
            4 => (nibble(0), nibble(1), nibble(2), nibble(3)),
            6 => (byte(0), byte(2), byte(4), Ok(255)),
            8 => (byte(0), byte(2), byte(4), byte(6)),
            _ => return Err(err()),
        };
        match parsed {
            (Ok(r), Ok(g), Ok(b), Ok(a)) => Ok(Self { r, g, b, a }),
            _ => Err(err()),
        }
    }

    /// Reads a colour the way colour scheme XML files write it: six hex
    /// digits (`RRGGBB`), eight (`RRGGBBAA`), or a shorter hex number whose
    /// missing high digits are zero (`0` is black). Leading `#` is accepted.
    pub fn parse_scheme(text: &str) -> Result<Self, ColorParseError> {
        let hex = text.trim().trim_start_matches('#');
        let err = || ColorParseError(text.to_owned());
        match hex.len() {
            // A number, not the CSS shorthand: `f` is blue, `abc` is 0x000abc.
            1..=5 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let value = u32::from_str_radix(hex, 16).map_err(|_| err())?;
                Ok(Self::rgb(
                    (value >> 16) as u8,
                    (value >> 8) as u8,
                    value as u8,
                ))
            }
            6 | 8 => Self::parse(hex),
            _ => Err(err()),
        }
    }

    /// `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn to_hex(self) -> String {
        if self.is_opaque() {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Composites `self` over an opaque background.
    pub fn over(self, background: Rgba) -> Rgba {
        if self.a == 255 {
            return self;
        }
        let a = f32::from(self.a) / 255.0;
        let mix = |fg: u8, bg: u8| (f32::from(fg) * a + f32::from(bg) * (1.0 - a)).round() as u8;
        Rgba::rgb(
            mix(self.r, background.r),
            mix(self.g, background.g),
            mix(self.b, background.b),
        )
    }

    /// WCAG relative luminance of the colour ignoring alpha.
    pub fn luminance(self) -> f64 {
        fn channel(value: u8) -> f64 {
            let c = f64::from(value) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// WCAG 2 contrast ratio between two colours; translucent colours are first
/// composited over `backdrop` (the colour behind the pair).
pub fn contrast_ratio(fg: Rgba, bg: Rgba, backdrop: Rgba) -> f64 {
    let bg = bg.over(backdrop);
    let fg = fg.over(bg);
    let (a, b) = (fg.luminance(), bg.luminance());
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl std::str::FromStr for Rgba {
    type Err = ColorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Rgba::parse(&text).map_err(D::Error::custom)
    }
}

/// A colour in a theme file: a literal or a `$name` reference to the
/// `[palette]` table of the theme or of one of its parents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorRef {
    Rgba(Rgba),
    Palette(String),
}

impl From<Rgba> for ColorRef {
    fn from(value: Rgba) -> Self {
        ColorRef::Rgba(value)
    }
}

impl Serialize for ColorRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ColorRef::Rgba(c) => c.serialize(serializer),
            ColorRef::Palette(name) => serializer.serialize_str(&format!("${name}")),
        }
    }
}

impl<'de> Deserialize<'de> for ColorRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match text.strip_prefix('$') {
            Some(name) if !name.is_empty() => Ok(ColorRef::Palette(name.to_owned())),
            Some(_) => Err(D::Error::custom("empty palette reference")),
            None => Rgba::parse(&text)
                .map(ColorRef::Rgba)
                .map_err(D::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Rgba::parse("#102030").unwrap(), Rgba::rgb(0x10, 0x20, 0x30));
        assert_eq!(
            Rgba::parse("102030ff").unwrap(),
            Rgba::rgb(0x10, 0x20, 0x30)
        );
        assert_eq!(Rgba::parse("#1020304d").unwrap().a, 0x4d);
        assert_eq!(Rgba::parse("#fa0").unwrap(), Rgba::rgb(0xff, 0xaa, 0x00));
        assert_eq!(
            Rgba::parse("#fa08").unwrap(),
            Rgba::new(0xff, 0xaa, 0x00, 0x88)
        );
        assert!(Rgba::parse("#12345").is_err());
        assert!(Rgba::parse("#gg0000").is_err());
        assert!(Rgba::parse("").is_err());
        assert!(Rgba::parse("#é12345").is_err());
    }

    #[test]
    fn scheme_numbers_pad_on_the_left() {
        assert_eq!(Rgba::parse_scheme("0").unwrap(), Rgba::BLACK);
        assert_eq!(Rgba::parse_scheme("ff").unwrap(), Rgba::rgb(0, 0, 255));
        assert_eq!(Rgba::parse_scheme("ff00").unwrap(), Rgba::rgb(0, 0xff, 0));
        assert_eq!(Rgba::parse_scheme("0000ff").unwrap(), Rgba::rgb(0, 0, 255));
        assert_eq!(
            Rgba::parse_scheme("112233").unwrap(),
            Rgba::rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(
            Rgba::parse_scheme("11223380").unwrap(),
            Rgba::new(0x11, 0x22, 0x33, 0x80)
        );
        assert!(Rgba::parse_scheme("").is_err());
        assert!(Rgba::parse_scheme("zz").is_err());
    }

    #[test]
    fn hex_round_trip() {
        for text in ["#000000", "#ffffff", "#12ab9f", "#12ab9f80"] {
            assert_eq!(Rgba::parse(text).unwrap().to_hex(), text);
        }
    }

    #[test]
    fn contrast_matches_wcag_reference_values() {
        let r = contrast_ratio(Rgba::BLACK, Rgba::WHITE, Rgba::WHITE);
        assert!((r - 21.0).abs() < 1e-9);
        let r = contrast_ratio(Rgba::WHITE, Rgba::WHITE, Rgba::WHITE);
        assert!((r - 1.0).abs() < 1e-9);
        // #767676 on white is the well known 4.54:1 boundary of AA.
        let r = contrast_ratio(Rgba::rgb(0x76, 0x76, 0x76), Rgba::WHITE, Rgba::WHITE);
        assert!((4.5..4.6).contains(&r), "{r}");
    }

    #[test]
    fn translucent_colours_are_composited() {
        let half_black = Rgba::new(0, 0, 0, 128);
        let over_white = half_black.over(Rgba::WHITE);
        assert!((i32::from(over_white.r) - 127).abs() <= 1);
        let r = contrast_ratio(Rgba::BLACK, half_black, Rgba::WHITE);
        assert!(r > 3.0 && r < 6.0, "{r}");
    }

    #[test]
    fn colour_ref_serde() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Holder {
            a: ColorRef,
            b: ColorRef,
        }
        let h: Holder = toml::from_str("a = \"#ff0000\"\nb = \"$red\"\n").unwrap();
        assert_eq!(h.a, ColorRef::Rgba(Rgba::rgb(255, 0, 0)));
        assert_eq!(h.b, ColorRef::Palette("red".into()));
        let text = toml::to_string(&h).unwrap();
        assert_eq!(toml::from_str::<Holder>(&text).unwrap(), h);
        assert!(toml::from_str::<Holder>("a = \"$\"\nb = \"#fff\"").is_err());
    }
}
