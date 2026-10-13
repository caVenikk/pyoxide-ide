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

const ESC: u8 = 0x1b;

/// The SGR state of one stream (the reference's `AnsiTerminalEmulator`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Sgr {
    font: u8,
    weight: Weight,
    underline: Underline,
    blink: u8,
    frame: u8,
    italic: bool,
    inverse: bool,
    conceal: bool,
    crossed_out: bool,
    fraktur: bool,
    overlined: bool,
    fg: Option<Color>,
    bg: Option<Color>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Weight {
    #[default]
    Normal,
    Bold,
    Faint,
}

impl Sgr {
    /// The style of text printed in this state.
    pub(crate) fn style(&self) -> Style {
        if *self == Sgr::default() {
            return Style::default();
        }
        Style {
            fg: self.fg,
            bg: self.bg,
            bold: self.weight == Weight::Bold,
            dim: self.weight == Weight::Faint,
            italic: self.italic,
            underline: self.underline,
            inverse: self.inverse,
            strikethrough: self.crossed_out,
            ansi: true,
        }
    }

    /// Applies the body of an SGR sequence (between `ESC [` and `m`).
    /// `:` separates the codes when the body contains one, `;` otherwise; an
    /// empty code is 0; a code that is not a number ends the sequence.
    pub(crate) fn apply(&mut self, body: &str) {
        let separator = if body.contains(':') { ':' } else { ';' };
        let mut codes = body.split(separator).map(parse_code);
        while let Some(Some(code)) = codes.next() {
            self.apply_code(code, &mut codes);
        }
    }

    fn apply_code(&mut self, code: i32, rest: &mut impl Iterator<Item = Option<i32>>) {
        match code {
            0 => *self = Sgr::default(),
            1 => self.weight = Weight::Bold,
            2 => self.weight = Weight::Faint,
            3 => self.italic = true,
            4 => self.underline = Underline::Single,
            5 => self.blink = 1,
            6 => self.blink = 2,
            7 => self.inverse = true,
            8 => self.conceal = true,
            9 => self.crossed_out = true,
            10 => self.font = 0,
            11..=19 => self.font = (code - 10) as u8,
            20 => self.fraktur = true,
            21 => {
                // ECMA-48: double underline, and the weight is reset.
                self.weight = Weight::Normal;
                self.underline = Underline::Double;
            }
            22 => self.weight = Weight::Normal,
            23 => {
                self.fraktur = false;
                self.italic = false;
            }
            24 => self.underline = Underline::None,
            25 => self.blink = 0,
            27 => self.inverse = false,
            28 => self.conceal = false,
            29 => self.crossed_out = false,
            30..=37 => self.fg = Some(Color::Ansi((code - 30) as u8)),
            38 => self.fg = decode_color(rest),
            39 => self.fg = None,
            40..=47 => self.bg = Some(Color::Ansi((code - 40) as u8)),
            48 => self.bg = decode_color(rest),
            49 => self.bg = None,
            51 => self.frame = 1,
            52 => self.frame = 2,
            53 => self.overlined = true,
            54 => self.frame = 0,
            55 => self.overlined = false,
            90..=97 => self.fg = Some(Color::Ansi((code - 90 + 8) as u8)),
            100..=107 => self.bg = Some(Color::Ansi((code - 100 + 8) as u8)),
            // 60-65 (ideograms) and unknown codes change nothing.
            _ => {}
        }
    }
}

/// `Integer.parseInt` of a code, an empty code being 0.
fn parse_code(code: &str) -> Option<i32> {
    if code.is_empty() {
        return Some(0);
    }
    code.parse::<i32>().ok()
}

/// The colour after 38 or 48: `5;n` or `2;r;g;b`. Anything else gives the
/// default colour, as the reference does, using up only what it read.
fn decode_color(rest: &mut impl Iterator<Item = Option<i32>>) -> Option<Color> {
    match rest.next()?? {
        2 => {
            let r = rest.next()??;
            let g = rest.next()??;
            let b = rest.next()??;
            Some(Color::Rgb(
                u8::try_from(r).ok()?,
                u8::try_from(g).ok()?,
                u8::try_from(b).ok()?,
            ))
        }
        5 => match rest.next()?? {
            n @ 0..=15 => Some(Color::Ansi(n as u8)),
            n @ 16..=255 => Some(Color::Indexed(n as u8)),
            _ => None,
        },
        _ => None,
    }
}

/// The streaming escape sequence lexer of one stream (the reference's
/// `AnsiStreamingLexer`): text, SGR sequences, other control sequences
/// (dropped), and an incomplete sequence at the end kept for the next chunk.
#[derive(Debug, Default)]
pub(crate) struct Lexer {
    pending: String,
}

impl Lexer {
    /// Splits `text` into styled pieces, applying SGR sequences to `sgr`.
    /// `emit` gets each piece of text with its style.
    pub(crate) fn feed(&mut self, text: &str, sgr: &mut Sgr, mut emit: impl FnMut(&str, Style)) {
        let joined;
        let s = if self.pending.is_empty() {
            text
        } else {
            let mut j = std::mem::take(&mut self.pending);
            j.push_str(text);
            joined = j;
            joined.as_str()
        };
        let b = s.as_bytes();
        let len = b.len();
        let next_esc = |from: usize| -> usize {
            b[from.min(len)..]
                .iter()
                .position(|c| *c == ESC)
                .map_or(len, |i| from + i)
        };
        let mut pos = 0;
        while pos < len {
            if b[pos] != ESC {
                let end = next_esc(pos);
                emit(&s[pos..end], sgr.style());
                pos = end;
                continue;
            }
            if pos + 1 == len {
                self.pending.push_str(&s[pos..]);
                return;
            }
            match b[pos + 1] {
                b'[' => {
                    let mut i = pos + 2;
                    while i < len && (0x30..=0x3f).contains(&b[i]) {
                        i += 1;
                    }
                    while i < len && (0x20..=0x2f).contains(&b[i]) {
                        i += 1;
                    }
                    if i == len {
                        self.pending.push_str(&s[pos..]);
                        return;
                    }
                    let last = b[i];
                    if last == b'm' {
                        sgr.apply(&s[pos + 2..i]);
                        pos = i + 1;
                    } else if (0x40..=0x7e).contains(&last) {
                        // Other CSI sequences are dropped.
                        pos = i + 1;
                    } else {
                        // A broken sequence is text, up to the next escape.
                        let end = next_esc(i + 1);
                        emit(&s[pos..end], sgr.style());
                        pos = end;
                    }
                }
                // DECKPAM and DECKPNM are dropped.
                b'=' | b'>' => pos += 2,
                _ => {
                    // Any other escape sequence (OSC included) stays text.
                    let end = next_esc(pos + 1);
                    emit(&s[pos..end], sgr.style());
                    pos = end;
                }
            }
        }
    }

    /// Drops an incomplete sequence (the reference never completes it).
    pub(crate) fn finish(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(chunks: &[&str]) -> Vec<(String, Style)> {
        let mut lexer = Lexer::default();
        let mut sgr = Sgr::default();
        let mut out: Vec<(String, Style)> = Vec::new();
        for chunk in chunks {
            lexer.feed(chunk, &mut sgr, |text, style| match out.last_mut() {
                Some((t, s)) if *s == style => t.push_str(text),
                _ => out.push((text.to_owned(), style)),
            });
        }
        out
    }

    fn sgr(body: &str) -> Style {
        let mut sgr = Sgr::default();
        sgr.apply(body);
        sgr.style()
    }

    fn plain(text: &str) -> (String, Style) {
        (text.to_owned(), Style::default())
    }

    #[test]
    fn plain_text_has_the_default_style() {
        assert_eq!(lex(&["hello"]), vec![plain("hello")]);
    }

    #[test]
    fn sixteen_colours_and_reset() {
        let red = Style {
            fg: Some(Color::Ansi(1)),
            ansi: true,
            ..Style::default()
        };
        assert_eq!(
            lex(&["a\x1b[31mred\x1b[0mb"]),
            vec![plain("a"), ("red".into(), red), plain("b")]
        );
        assert_eq!(sgr("91").fg, Some(Color::Ansi(9)));
        assert_eq!(sgr("44").bg, Some(Color::Ansi(4)));
        assert_eq!(sgr("103").bg, Some(Color::Ansi(11)));
        assert_eq!(sgr("31;39"), Style::default());
        assert_eq!(sgr("41;49"), Style::default());
        // An empty code is 0: `ESC [ m` and a trailing `;` reset.
        assert_eq!(sgr("1;"), Style::default());
        let mut s = Sgr::default();
        s.apply("1");
        s.apply("");
        assert_eq!(s.style(), Style::default());
    }

    #[test]
    fn extended_colours() {
        assert_eq!(sgr("38;5;3").fg, Some(Color::Ansi(3)));
        assert_eq!(sgr("38;5;196").fg, Some(Color::Indexed(196)));
        assert_eq!(sgr("48;5;232").bg, Some(Color::Indexed(232)));
        assert_eq!(sgr("38;2;10;20;30").fg, Some(Color::Rgb(10, 20, 30)));
        assert_eq!(sgr("48:2:1:2:3").bg, Some(Color::Rgb(1, 2, 3)));
        // Invalid colours fall back to the default.
        assert_eq!(sgr("31;38;5;300").fg, None);
        assert_eq!(sgr("31;38;2;1;2").fg, None);
        // `38;9` uses up `9` only; `1` is then bold.
        let s = sgr("38;9;1");
        assert_eq!(s.fg, None);
        assert!(s.bold);
    }

    #[test]
    fn attributes() {
        let s = sgr("1;3;4;7;9");
        assert!(s.bold && s.italic && s.inverse && s.strikethrough && s.ansi);
        assert_eq!(s.underline, Underline::Single);
        assert!(sgr("2").dim);
        let s = sgr("1;21");
        assert!(!s.bold);
        assert_eq!(s.underline, Underline::Double);
        for off in [
            "1;22", "3;23", "4;24", "7;27", "9;29", "5;25", "8;28", "53;55",
        ] {
            assert_eq!(sgr(off), Style::default(), "{off}");
        }
        // Attributes that are not drawn still switch to ANSI drawing.
        let blink = sgr("5");
        assert!(blink.ansi);
        assert_eq!(
            Style {
                ansi: false,
                ..blink
            },
            Style::default()
        );
        // Unknown codes change nothing; a code that is not a number ends the
        // sequence.
        assert_eq!(sgr("99"), Style::default());
        assert_eq!(sgr("?1;31"), Style::default());
        let s = sgr("31;?1;1");
        assert_eq!(s.fg, Some(Color::Ansi(1)));
        assert!(!s.bold);
    }

    #[test]
    fn other_sequences() {
        // Other CSI sequences, ESC = and ESC > are dropped.
        assert_eq!(
            lex(&["a\x1b[2Kb\x1b[?25lc\x1b=d\x1b>e"]),
            vec![plain("abcde")]
        );
        // OSC and other escapes stay text.
        assert_eq!(
            lex(&["\x1b]0;title\x07x"]),
            vec![plain("\x1b]0;title\x07x")]
        );
        // A broken CSI sequence is text up to the next escape.
        let bold = Style {
            bold: true,
            ansi: true,
            ..Style::default()
        };
        assert_eq!(
            lex(&["\x1b[12\nab\x1b[1mc"]),
            vec![plain("\x1b[12\nab"), ("c".into(), bold)]
        );
    }

    #[test]
    fn sequences_split_between_chunks() {
        let green = Style {
            fg: Some(Color::Ansi(2)),
            ansi: true,
            ..Style::default()
        };
        let text = "\x1b[32mok\x1b[0m";
        for split in 1..text.len() {
            let (a, b) = text.split_at(split);
            assert_eq!(
                lex(&["x", a, b]),
                vec![plain("x"), ("ok".into(), green)],
                "split at {split}"
            );
        }
        let mut lexer = Lexer::default();
        let mut sgr = Sgr::default();
        let mut out = String::new();
        lexer.feed("a\x1b[3", &mut sgr, |t, _| out.push_str(t));
        lexer.finish();
        lexer.feed("1mb", &mut sgr, |t, _| out.push_str(t));
        assert_eq!(out, "a1mb");
    }

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
