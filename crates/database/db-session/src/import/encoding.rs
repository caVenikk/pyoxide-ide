//! Text encodings of import files: UTF-8 (with or without a byte order
//! mark), UTF-16 and Windows-1252, found from the start of the file and
//! decoded while reading, so a file is never read whole.

use std::io::{self, BufRead, BufReader, Cursor, Read};

use serde::{Deserialize, Serialize};

/// The encodings an import file can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    /// Windows-1252, what a file that is not UTF-8 is most likely in.
    Windows1252,
}

impl Encoding {
    pub const ALL: [Encoding; 4] = [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Windows1252,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
            Encoding::Windows1252 => "windows-1252",
        }
    }

    pub fn from_title(title: &str) -> Option<Encoding> {
        Encoding::ALL
            .into_iter()
            .find(|e| e.title().eq_ignore_ascii_case(title))
    }

    /// Bytes of the byte order mark of this encoding at the start of `data`.
    fn bom_len(self, data: &[u8]) -> usize {
        let bom: &[u8] = match self {
            Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            Encoding::Windows1252 => &[],
        };
        if data.starts_with(bom) { bom.len() } else { 0 }
    }
}

/// The encoding of a file from its first bytes, with the length of its byte
/// order mark: a mark decides; else valid UTF-8 (a character cut at the end
/// of the sample is fine); else UTF-16 when every other byte is zero; else
/// Windows-1252.
pub fn detect(sample: &[u8]) -> (Encoding, usize) {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (Encoding::Utf8, 3);
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return (Encoding::Utf16Le, 2);
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return (Encoding::Utf16Be, 2);
    }
    match std::str::from_utf8(sample) {
        Ok(_) => {}
        Err(e) if e.error_len().is_none() => {}
        Err(_) => return (Encoding::Windows1252, 0),
    }
    if sample.len() >= 4 {
        let even_zero = sample.iter().step_by(2).filter(|b| **b == 0).count();
        let odd_zero = sample
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|b| **b == 0)
            .count();
        let pairs = sample.len() / 2;
        if odd_zero * 2 > pairs && even_zero == 0 {
            return (Encoding::Utf16Le, 0);
        }
        if even_zero * 2 > pairs && odd_zero == 0 {
            return (Encoding::Utf16Be, 0);
        }
    }
    (Encoding::Utf8, 0)
}

/// How much of the start of a file is looked at to find its encoding.
const SAMPLE: usize = 64 * 1024;

/// Opens `reader` as UTF-8 text: the encoding is `forced`, or detected from
/// the start of the stream. Returns the reader and the encoding used.
pub fn open<R: Read + Send + 'static>(
    mut reader: R,
    forced: Option<Encoding>,
) -> io::Result<(Box<dyn BufRead + Send>, Encoding)> {
    let mut sample = vec![0u8; SAMPLE];
    let mut n = 0;
    while n < sample.len() {
        let got = reader.read(&mut sample[n..])?;
        if got == 0 {
            break;
        }
        n += got;
    }
    sample.truncate(n);
    let (encoding, bom) = match forced {
        Some(e) => (e, e.bom_len(&sample)),
        None => detect(&sample),
    };
    let rest = Cursor::new(sample.split_off(bom)).chain(reader);
    let reader: Box<dyn BufRead + Send> = match encoding {
        Encoding::Utf8 => Box::new(BufReader::with_capacity(1 << 16, rest)),
        Encoding::Utf16Le => Box::new(BufReader::new(Utf16Reader::new(rest, false))),
        Encoding::Utf16Be => Box::new(BufReader::new(Utf16Reader::new(rest, true))),
        Encoding::Windows1252 => Box::new(BufReader::new(Windows1252Reader::new(rest))),
    };
    Ok((reader, encoding))
}

/// Turns UTF-16 into UTF-8 as it is read.
struct Utf16Reader<R> {
    inner: R,
    big_endian: bool,
    /// A byte left over from an odd read.
    odd: Option<u8>,
    /// A high surrogate waiting for its low one.
    high: Option<u16>,
    out: Vec<u8>,
    at: usize,
}

impl<R: Read> Utf16Reader<R> {
    fn new(inner: R, big_endian: bool) -> Self {
        Self {
            inner,
            big_endian,
            odd: None,
            high: None,
            out: Vec::new(),
            at: 0,
        }
    }

    fn push(&mut self, c: char) {
        let mut b = [0u8; 4];
        self.out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
    }

    fn unit(&mut self, u: u16) {
        if let Some(h) = self.high.take() {
            if (0xDC00..0xE000).contains(&u) {
                let c = 0x10000 + ((u32::from(h) - 0xD800) << 10) + (u32::from(u) - 0xDC00);
                self.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                return;
            }
            self.push('\u{FFFD}');
        }
        if (0xD800..0xDC00).contains(&u) {
            self.high = Some(u);
        } else {
            self.push(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'));
        }
    }

    fn refill(&mut self) -> io::Result<bool> {
        let mut raw = [0u8; 8192];
        let mut start = 0;
        if let Some(b) = self.odd.take() {
            raw[0] = b;
            start = 1;
        }
        let got = self.inner.read(&mut raw[start..])?;
        let total = start + got;
        if got == 0 {
            if let Some(_h) = self.high.take() {
                self.push('\u{FFFD}');
            }
            return Ok(false);
        }
        let usable = total & !1;
        if total % 2 == 1 {
            self.odd = Some(raw[total - 1]);
        }
        for pair in raw[..usable].chunks_exact(2) {
            let u = if self.big_endian {
                u16::from_be_bytes([pair[0], pair[1]])
            } else {
                u16::from_le_bytes([pair[0], pair[1]])
            };
            self.unit(u);
        }
        Ok(true)
    }
}

impl<R: Read> Read for Utf16Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.out.len() {
            self.out.clear();
            self.at = 0;
            loop {
                let more = self.refill()?;
                if !self.out.is_empty() || !more {
                    break;
                }
            }
            if self.out.is_empty() {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.out.len() - self.at);
        buf[..n].copy_from_slice(&self.out[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Windows-1252 as UTF-8.
struct Windows1252Reader<R> {
    inner: R,
    out: Vec<u8>,
    at: usize,
}

impl<R: Read> Windows1252Reader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            out: Vec::new(),
            at: 0,
        }
    }
}

/// The characters of 0x80..=0x9F in Windows-1252.
const HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

fn windows_1252(b: u8) -> char {
    match b {
        0x80..=0x9F => HIGH[usize::from(b - 0x80)],
        b => char::from(b),
    }
}

impl<R: Read> Read for Windows1252Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.out.len() {
            let mut raw = [0u8; 4096];
            let got = self.inner.read(&mut raw)?;
            if got == 0 {
                return Ok(0);
            }
            self.out.clear();
            self.at = 0;
            for &b in &raw[..got] {
                let mut tmp = [0u8; 4];
                self.out
                    .extend_from_slice(windows_1252(b).encode_utf8(&mut tmp).as_bytes());
            }
        }
        let n = buf.len().min(self.out.len() - self.at);
        buf[..n].copy_from_slice(&self.out[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: Vec<u8>, forced: Option<Encoding>) -> (String, Encoding) {
        let (mut r, e) = open(Cursor::new(bytes), forced).unwrap();
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        (s, e)
    }

    #[test]
    fn utf8_with_and_without_a_mark() {
        let (s, e) = text("a,б,日本\n".as_bytes().to_vec(), None);
        assert_eq!((s.as_str(), e), ("a,б,日本\n", Encoding::Utf8));
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice("x".as_bytes());
        assert_eq!(text(with_bom, None), ("x".into(), Encoding::Utf8));
    }

    #[test]
    fn utf16_either_way() {
        let units: Vec<u16> = "a,é,日本,😀\n".encode_utf16().collect();
        let mut le = vec![0xFF, 0xFE];
        le.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        assert_eq!(text(le, None), ("a,é,日本,😀\n".into(), Encoding::Utf16Le));
        let mut be = vec![0xFE, 0xFF];
        be.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        assert_eq!(text(be, None), ("a,é,日本,😀\n".into(), Encoding::Utf16Be));
        // No mark: found from the zero bytes.
        let bare: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        let ascii: Vec<u8> = "abcdef"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(text(ascii, None).1, Encoding::Utf16Le);
        let _ = bare;
    }

    #[test]
    fn windows_1252_when_not_utf8() {
        // "café – 5€" in Windows-1252.
        let bytes = vec![b'c', b'a', b'f', 0xE9, b' ', 0x96, b' ', b'5', 0x80];
        assert_eq!(
            text(bytes, None),
            ("café – 5€".into(), Encoding::Windows1252)
        );
    }

    #[test]
    fn a_forced_encoding_wins() {
        let (s, e) = text(vec![0xE9], Some(Encoding::Windows1252));
        assert_eq!((s.as_str(), e), ("é", Encoding::Windows1252));
    }

    #[test]
    fn a_character_cut_at_the_end_of_the_sample_is_still_utf8() {
        let mut bytes = vec![b'a'; SAMPLE - 1];
        bytes.extend_from_slice("日".as_bytes());
        assert_eq!(detect(&bytes[..SAMPLE]).0, Encoding::Utf8);
    }
}
