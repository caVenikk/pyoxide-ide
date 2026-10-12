//! Bytes on disk to text and back: encodings, byte order marks and line
//! separators. Text inside the buffer always uses `\n`; the separator and
//! encoding of the file are remembered and restored when it is written.

use std::borrow::Cow;

use serde::Serialize;

/// Line separator of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum LineEnding {
    Lf,
    Crlf,
    Cr,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    /// Name shown in the status bar.
    pub const fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }

    /// The first separator in `text`, if any.
    pub fn detect(text: &str) -> Option<LineEnding> {
        let bytes = text.as_bytes();
        let i = bytes.iter().position(|&b| b == b'\n' || b == b'\r')?;
        Some(match (bytes[i], bytes.get(i + 1)) {
            (b'\n', _) => LineEnding::Lf,
            (b'\r', Some(b'\n')) => LineEnding::Crlf,
            _ => LineEnding::Cr,
        })
    }
}

/// Text encoding of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Encoding {
    Utf8,
    /// UTF-8 that starts with a byte order mark.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    /// Name shown in the status bar.
    pub const fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
        }
    }
}

/// A file's bytes as editor text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Text with `\n` line breaks.
    pub text: String,
    /// The first separator of the file; LF for a file without one.
    pub line_ending: LineEnding,
    pub encoding: Encoding,
    /// Invalid byte sequences were replaced: saving would change the file,
    /// so such a document is opened read-only.
    pub lossy: bool,
}

/// Why bytes are not shown as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Looks like a binary file (NUL bytes and no UTF-16 byte order mark).
    Binary,
}

/// Bytes inspected for NUL bytes when detecting binary files.
const BINARY_PROBE: usize = 8000;

/// Decodes a file. UTF-8 (with or without BOM) and UTF-16 with a BOM are
/// read; other bytes are read as UTF-8 with replacement characters.
pub fn decode(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    let (encoding, text, lossy) = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        let (text, lossy) = utf8(rest);
        (Encoding::Utf8Bom, text, lossy)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        let (text, lossy) = utf16(rest, u16::from_le_bytes);
        (Encoding::Utf16Le, text, lossy)
    } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        let (text, lossy) = utf16(rest, u16::from_be_bytes);
        (Encoding::Utf16Be, text, lossy)
    } else {
        if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) {
            return Err(DecodeError::Binary);
        }
        let (text, lossy) = utf8(bytes);
        (Encoding::Utf8, text, lossy)
    };
    let line_ending = LineEnding::detect(&text).unwrap_or(LineEnding::Lf);
    let text = match normalize_line_endings(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(normalized) => normalized,
    };
    Ok(Decoded {
        text,
        line_ending,
        encoding,
        lossy,
    })
}

fn utf8(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

fn utf16(bytes: &[u8], read: fn([u8; 2]) -> u16) -> (String, bool) {
    let (units, rest) = bytes.as_chunks::<2>();
    let units: Vec<u16> = units.iter().map(|&pair| read(pair)).collect();
    let mut lossy = !rest.is_empty();
    let text = char::decode_utf16(units)
        .map(|c| {
            c.unwrap_or_else(|_| {
                lossy = true;
                char::REPLACEMENT_CHARACTER
            })
        })
        .collect();
    (text, lossy)
}

/// Replaces `\r\n` and lone `\r` with `\n`.
pub fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Encodes editor text (with `\n` breaks) for writing to disk.
pub fn encode(text: &str, line_ending: LineEnding, encoding: Encoding) -> Vec<u8> {
    let text: Cow<str> = match line_ending {
        LineEnding::Lf => Cow::Borrowed(text),
        other => Cow::Owned(text.replace('\n', other.as_str())),
    };
    match encoding {
        Encoding::Utf8 => text.into_owned().into_bytes(),
        Encoding::Utf8Bom => {
            let mut out = b"\xEF\xBB\xBF".to_vec();
            out.extend_from_slice(text.as_bytes());
            out
        }
        Encoding::Utf16Le => {
            let mut out = b"\xFF\xFE".to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf16Be => {
            let mut out = b"\xFE\xFF".to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_separators() {
        assert_eq!(LineEnding::detect("a\nb\r\n"), Some(LineEnding::Lf));
        assert_eq!(LineEnding::detect("a\r\nb\n"), Some(LineEnding::Crlf));
        assert_eq!(LineEnding::detect("a\rb"), Some(LineEnding::Cr));
        assert_eq!(LineEnding::detect("abc"), None);
    }

    #[test]
    fn round_trips_crlf_with_bom() {
        let bytes = b"\xEF\xBB\xBFone\r\ntwo\r\n";
        let d = decode(bytes).unwrap();
        assert_eq!(d.text, "one\ntwo\n");
        assert_eq!(d.line_ending, LineEnding::Crlf);
        assert_eq!(d.encoding, Encoding::Utf8Bom);
        assert!(!d.lossy);
        assert_eq!(encode(&d.text, d.line_ending, d.encoding), bytes);
    }

    #[test]
    fn mixed_separators_are_saved_with_the_first_one() {
        let d = decode(b"a\r\nb\nc\rd").unwrap();
        assert_eq!(d.text, "a\nb\nc\nd");
        assert_eq!(
            encode(&d.text, d.line_ending, d.encoding),
            b"a\r\nb\r\nc\r\nd"
        );
    }

    #[test]
    fn reads_utf16_with_bom() {
        let mut le = vec![0xFF, 0xFE];
        le.extend("h\u{e9}\n".encode_utf16().flat_map(u16::to_le_bytes));
        let d = decode(&le).unwrap();
        assert_eq!(
            (d.text.as_str(), d.encoding),
            ("h\u{e9}\n", Encoding::Utf16Le)
        );
        assert_eq!(encode(&d.text, d.line_ending, d.encoding), le);
        let mut be = vec![0xFE, 0xFF];
        be.extend("x".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode(&be).unwrap().encoding, Encoding::Utf16Be);
    }

    #[test]
    fn binary_and_invalid_utf8() {
        assert_eq!(decode(b"PK\x03\x04\0\0"), Err(DecodeError::Binary));
        let d = decode(b"ok \xFF here").unwrap();
        assert!(d.lossy);
        assert_eq!(d.text, "ok \u{FFFD} here");
        let empty = decode(b"").unwrap();
        assert_eq!(
            (empty.text.as_str(), empty.line_ending),
            ("", LineEnding::Lf)
        );
    }
}
