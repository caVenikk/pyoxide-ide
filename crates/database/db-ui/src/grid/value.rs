//! What a grid shows for a value (scenario grid-values): the text of a cell
//! and the text of the value panel.
//!
//! A cell shows one line: `<null>` for SQL NULL (grey, unlike an empty
//! string, which is blank), exact numbers in the digits the server sent,
//! dates and timestamps in the patterns of Settings > Database > Data Views
//! ([`DataFormats`]), a PostgreSQL interval in words, JSON as the server
//! returned it, arrays as `[1, 2, 3]`. A value the page cut at the fetch
//! limit shows its prefix. Binary values follow the column's display type
//! (the cell menu's Change Display Type): hex, hex and ASCII, text, or a
//! UUID.
//!
//! The panel shows the whole value: text as it is, JSON optionally
//! indented (the text is re-flowed token by token, so numbers and the order
//! of keys stay as the server wrote them), binary as a hex dump.

use std::collections::HashMap;
use std::sync::Arc;

use db_core::Value;

use super::format;
use super::formats::DataFormats;

/// How a binary column's values are shown (Change Display Type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplayType {
    /// A UUID for 16 bytes, hex for anything else.
    #[default]
    Detect,
    Uuid,
    /// The byte order of a GUID as SQL Server and .NET keep it: the first
    /// three groups are little-endian.
    UuidSwapped,
    Text,
    Hex,
    HexAscii,
}

impl DisplayType {
    pub const ALL: [DisplayType; 6] = [
        DisplayType::Detect,
        DisplayType::Uuid,
        DisplayType::UuidSwapped,
        DisplayType::Text,
        DisplayType::Hex,
        DisplayType::HexAscii,
    ];

    /// The id of the command's argument.
    pub fn id(self) -> &'static str {
        match self {
            DisplayType::Detect => "detect",
            DisplayType::Uuid => "uuid",
            DisplayType::UuidSwapped => "uuid-swapped",
            DisplayType::Text => "text",
            DisplayType::Hex => "hex",
            DisplayType::HexAscii => "hex-ascii",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id() == id)
    }

    /// The menu's label.
    pub fn title(self) -> &'static str {
        match self {
            DisplayType::Detect => "Detect",
            DisplayType::Uuid => "UUID",
            DisplayType::UuidSwapped => "UUID with time-low and time-high swapped",
            DisplayType::Text => "Text",
            DisplayType::Hex => "Hex",
            DisplayType::HexAscii => "Hex and ASCII",
        }
    }
}

/// What the cell texts of a page are made with: the formats of the
/// settings and the display type of each binary column (by name).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellFormat {
    pub formats: Arc<DataFormats>,
    pub display: HashMap<String, DisplayType>,
}

impl CellFormat {
    pub fn display_of(&self, column: &str) -> DisplayType {
        self.display.get(column).copied().unwrap_or_default()
    }
}

/// Longest cell text kept, in characters (a cell is one line of at most a
/// screen; the panel reads the value itself).
pub const CELL_TEXT_CAP: usize = 2000;

/// The cell text of `value`; `None` for SQL NULL.
pub fn cell_text(value: &Value, f: &DataFormats, binary: DisplayType) -> Option<String> {
    let mut text = match value {
        Value::Null => return None,
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => f.number(&n.to_string()),
        Value::Float(x) => f.number(&x.to_string()),
        Value::Decimal(s) => f.number(s),
        Value::Text(s) | Value::Json(s) | Value::Uuid(s) => s.clone(),
        Value::Bytes(b) => binary_text(b, binary, true),
        Value::Date(d) => f.format_date(*d),
        Value::Time(t) => f.format_time(*t),
        Value::Timestamp(t) => f.format_timestamp(*t),
        Value::TimestampTz(t) => f.format_timestamp_tz(*t),
        Value::Interval(text) => format::interval_words(text).unwrap_or_else(|| text.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|v| cell_text(v, f, binary).unwrap_or_else(|| "null".to_owned()))
                .collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Truncated { prefix, .. } => cell_text(prefix, f, binary).unwrap_or_default(),
        Value::Other { text, .. } => text.clone(),
    };
    if let Some((cut, _)) = text.char_indices().nth(CELL_TEXT_CAP) {
        text.truncate(cut);
    }
    Some(text)
}

/// Whether the value is cut: it has more than the page holds.
pub fn is_truncated(value: &Value) -> bool {
    matches!(value, Value::Truncated { .. })
}

/// How much of a value the page holds and how much there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// Bytes of the prefix.
    pub shown: usize,
    /// The whole length when the backend says it (bytes; Oracle counts the
    /// characters of a CLOB).
    pub full_len: Option<u64>,
}

/// What the page cut off a value, `None` for a whole value.
pub fn cut_of(value: &Value) -> Option<Cut> {
    let Value::Truncated { prefix, full_len } = value else {
        return None;
    };
    let shown = match prefix.as_ref() {
        Value::Text(s) | Value::Json(s) => s.len(),
        Value::Bytes(b) => b.len(),
        other => other.to_string().len(),
    };
    Some(Cut {
        shown,
        full_len: *full_len,
    })
}

/// The bytes of a binary value or its prefix.
pub fn bytes_of(value: &Value) -> Option<&[u8]> {
    match value {
        Value::Bytes(b) => Some(b),
        Value::Truncated { prefix, .. } => bytes_of(prefix),
        _ => None,
    }
}

/// `bytes` as the display type shows them; `line`: one line for a cell
/// (hex of the first bytes), not a dump.
pub fn binary_text(bytes: &[u8], display: DisplayType, line: bool) -> String {
    let uuid = |swap: bool| <[u8; 16]>::try_from(bytes).ok().map(|b| uuid_text(b, swap));
    match display {
        DisplayType::Detect => uuid(false).unwrap_or_else(|| hex_text(bytes, line)),
        DisplayType::Uuid => uuid(false).unwrap_or_else(|| hex_text(bytes, line)),
        DisplayType::UuidSwapped => uuid(true).unwrap_or_else(|| hex_text(bytes, line)),
        DisplayType::Text => String::from_utf8_lossy(bytes).into_owned(),
        DisplayType::Hex => hex_text(bytes, line),
        DisplayType::HexAscii if line => {
            let shown = &bytes[..bytes.len().min(CELL_TEXT_CAP / 4)];
            let ascii: String = shown.iter().map(|b| printable(*b)).collect();
            let hex: Vec<String> = shown.iter().map(|b| format!("{b:02X}")).collect();
            format!("{}  {ascii}", hex.join(" "))
        }
        DisplayType::HexAscii => dump(bytes),
    }
}

fn hex_text(bytes: &[u8], line: bool) -> String {
    let shown = if line {
        &bytes[..bytes.len().min(CELL_TEXT_CAP / 2)]
    } else {
        bytes
    };
    let mut out = String::with_capacity(2 + shown.len() * 2);
    out.push_str("0x");
    for b in shown {
        out.push_str(&format!("{b:02X}"));
    }
    out
}

fn printable(b: u8) -> char {
    if (0x20..0x7f).contains(&b) {
        b as char
    } else {
        '.'
    }
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`; swapped: the first three groups
/// read from the bytes in the opposite order.
pub fn uuid_text(mut b: [u8; 16], swapped: bool) -> String {
    if swapped {
        b[0..4].reverse();
        b[4..6].reverse();
        b[6..8].reverse();
    }
    let h = |r: std::ops::Range<usize>| b[r].iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        h(0..4),
        h(4..6),
        h(6..8),
        h(8..10),
        h(10..16)
    )
}

/// A hex dump: offset, sixteen bytes, their ASCII.
pub fn dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 + 16);
    for (i, chunk) in bytes.chunks(16).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{:08x} ", i * 16));
        for k in 0..16 {
            if k == 8 {
                out.push(' ');
            }
            match chunk.get(k) {
                Some(b) => out.push_str(&format!(" {b:02x}")),
                None => out.push_str("   "),
            }
        }
        out.push_str("  |");
        out.extend(chunk.iter().map(|b| printable(*b)));
        out.push('|');
    }
    out
}

/// The value panel's text: the whole value in the display type. `None` for
/// SQL NULL.
pub fn panel_text(value: &Value, f: &DataFormats, binary: DisplayType) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Truncated { prefix, .. } => panel_text(prefix, f, binary),
        Value::Bytes(b) => Some(match binary {
            DisplayType::Hex => hex_text(b, false),
            DisplayType::HexAscii => dump(b),
            other => binary_text(b, other, false),
        }),
        Value::Text(s) | Value::Json(s) | Value::Uuid(s) | Value::Decimal(s) => Some(s.clone()),
        Value::Other { text, .. } => Some(text.clone()),
        Value::Array(_) => {
            // The cell's text is cut for the cell; the panel has all of it.
            let Value::Array(items) = value else {
                unreachable!()
            };
            let parts: Vec<String> = items
                .iter()
                .map(|v| panel_text(v, f, binary).unwrap_or_else(|| "null".to_owned()))
                .collect();
            Some(format!("[{}]", parts.join(", ")))
        }
        other => cell_text(other, f, binary),
    }
}

/// Whether the panel's text may be shown as indented JSON: JSON values and
/// text that is a JSON document.
pub fn is_json(value: &Value) -> bool {
    match value {
        Value::Json(_) => true,
        Value::Text(s) => looks_like_json(s),
        Value::Truncated { prefix, .. } => is_json(prefix),
        _ => false,
    }
}

fn looks_like_json(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))
}

/// `text` as an indented JSON document (two spaces, one member a line,
/// `"key": value`); `None` when it is not balanced JSON. The tokens are
/// copied, so numbers keep their digits and members their order.
pub fn pretty_json(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    let mut stack: Vec<char> = Vec::new();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', depth * 2));
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                loop {
                    match chars.next()? {
                        '\\' => {
                            out.push('\\');
                            out.push(chars.next()?);
                        }
                        '"' => {
                            out.push('"');
                            break;
                        }
                        other => out.push(other),
                    }
                }
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                stack.push(close);
                out.push(c);
                // An empty container stays on one line.
                let mut ahead = chars.clone();
                while ahead.peek().is_some_and(|x| x.is_whitespace()) {
                    ahead.next();
                }
                if ahead.peek() == Some(&close) {
                    for _ in chars.by_ref().take_while(|x| *x != close) {}
                    out.push(close);
                    stack.pop();
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                if stack.pop() != Some(c) {
                    return None;
                }
                depth -= 1;
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    stack.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, TimeZone as _};

    use super::*;

    fn text(v: &Value) -> Option<String> {
        cell_text(v, &DataFormats::default(), DisplayType::Detect)
    }

    #[test]
    fn null_differs_from_an_empty_string() {
        assert_eq!(text(&Value::Null), None);
        assert_eq!(text(&Value::Text(String::new())), Some(String::new()));
    }

    #[test]
    fn numbers_stay_exact() {
        let big = "123456789012345678901234567890.1234567890";
        assert_eq!(text(&Value::Decimal(big.into())).unwrap(), big);
        assert_eq!(text(&Value::Int(i64::MIN)).unwrap(), "-9223372036854775808");
        assert_eq!(text(&Value::Float(0.998)).unwrap(), "0.998");
        let grouped = DataFormats {
            group_numbers: true,
            group_separator: ",".into(),
            ..DataFormats::default()
        };
        assert_eq!(
            cell_text(&Value::Decimal(big.into()), &grouped, DisplayType::Detect).unwrap(),
            "123,456,789,012,345,678,901,234,567,890.1234567890"
        );
    }

    #[test]
    fn dates_follow_the_settings() {
        let at = NaiveDate::from_ymd_opt(2026, 9, 25)
            .unwrap()
            .and_hms_opt(23, 54, 0)
            .unwrap();
        let tz = FixedOffset::east_opt(3 * 3600)
            .unwrap()
            .from_utc_datetime(&at);
        assert_eq!(
            text(&Value::TimestampTz(tz)).unwrap(),
            "2026-09-26 02:54:00.000000 +03:00"
        );
        let f = DataFormats {
            timestamp: "dd.MM.yyyy HH:mm".into(),
            date: "dd/MM/yy".into(),
            ..DataFormats::default()
        };
        assert_eq!(
            cell_text(&Value::Timestamp(at), &f, DisplayType::Detect).unwrap(),
            "25.09.2026 23:54"
        );
        assert_eq!(
            cell_text(&Value::Date(at.date()), &f, DisplayType::Detect).unwrap(),
            "25/09/26"
        );
    }

    #[test]
    fn composite_values_are_text() {
        let a = Value::Array(vec![Value::Int(1), Value::Null, Value::Text("x".into())]);
        assert_eq!(text(&a).unwrap(), "[1, null, x]");
        assert_eq!(
            text(&Value::Interval("00:04:58".into())).unwrap(),
            "4 mins 58 secs"
        );
        let u = "1f15485f-4b8a-bb91-e4cc-16e720e02c1e";
        assert_eq!(text(&Value::Uuid(u.into())).unwrap(), u);
        assert_eq!(
            text(&Value::Json("{\"a\": 1}".into())).unwrap(),
            "{\"a\": 1}"
        );
        let cut = Value::Truncated {
            prefix: Box::new(Value::Text("abc".into())),
            full_len: Some(10),
        };
        assert_eq!(text(&cut).unwrap(), "abc");
        assert!(is_truncated(&cut));
    }

    #[test]
    fn a_cell_keeps_a_screen_of_text() {
        let long = "x".repeat(CELL_TEXT_CAP * 3);
        assert_eq!(
            text(&Value::Text(long.clone())).unwrap().len(),
            CELL_TEXT_CAP
        );
        // The panel has all of it.
        let all = panel_text(
            &Value::Text(long.clone()),
            &DataFormats::default(),
            DisplayType::Detect,
        );
        assert_eq!(all.unwrap().len(), long.len());
    }

    #[test]
    fn binary_modes() {
        let raw: Vec<u8> = (0u8..16).collect();
        let b = Value::Bytes(raw.clone());
        let show = |d| cell_text(&b, &DataFormats::default(), d).unwrap();
        // 16 bytes are a UUID by detection.
        assert_eq!(
            show(DisplayType::Detect),
            "00010203-0405-0607-0809-0a0b0c0d0e0f"
        );
        assert_eq!(
            show(DisplayType::Uuid),
            "00010203-0405-0607-0809-0a0b0c0d0e0f"
        );
        assert_eq!(
            show(DisplayType::UuidSwapped),
            "03020100-0504-0706-0809-0a0b0c0d0e0f"
        );
        assert_eq!(show(DisplayType::Hex), "0x000102030405060708090A0B0C0D0E0F");
        assert!(show(DisplayType::HexAscii).starts_with("00 01 02"));
        assert!(show(DisplayType::HexAscii).ends_with("  ................"));
        let hello = Value::Bytes(b"Hello".to_vec());
        let f = DataFormats::default();
        // Other lengths are hex by detection, text on request.
        assert_eq!(
            cell_text(&hello, &f, DisplayType::Detect).unwrap(),
            "0x48656C6C6F"
        );
        assert_eq!(cell_text(&hello, &f, DisplayType::Text).unwrap(), "Hello");
        assert_eq!(
            cell_text(&hello, &f, DisplayType::Uuid).unwrap(),
            "0x48656C6C6F"
        );
        assert_eq!(
            cell_text(&hello, &f, DisplayType::HexAscii).unwrap(),
            "48 65 6C 6C 6F  Hello"
        );
    }

    #[test]
    fn a_hex_dump_has_offsets_and_ascii() {
        let d = dump(b"Hello, world! 0123456789");
        let lines: Vec<&str> = d.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 20 30 31  |Hello, world! 01|"
        );
        assert!(lines[1].starts_with("00000010  32 33 34 35 36 37 38 39"));
        assert!(lines[1].ends_with("|23456789|"));
    }

    #[test]
    fn json_is_indented_without_changing_its_tokens() {
        let src = r#"{"n": 1, "ok": false, "path": "/p/1", "nested": {"a": 1, "list": [1, 2, 1]}}"#;
        let want = "{\n  \"n\": 1,\n  \"ok\": false,\n  \"path\": \"/p/1\",\n  \"nested\": {\n    \"a\": 1,\n    \"list\": [\n      1,\n      2,\n      1\n    ]\n  }\n}";
        assert_eq!(pretty_json(src).unwrap(), want);
        // Strings keep their commas, braces, escapes and spaces.
        assert_eq!(
            pretty_json(r#"{"k": "a, {b} \" c"}"#).unwrap(),
            "{\n  \"k\": \"a, {b} \\\" c\"\n}"
        );
        // Big numbers are not parsed, empty containers stay short.
        assert_eq!(
            pretty_json("[12345678901234567890123, {}, []]").unwrap(),
            "[\n  12345678901234567890123,\n  {},\n  []\n]"
        );
        assert_eq!(pretty_json("{\"a\": 1"), None);
        assert_eq!(pretty_json("{\"a\": 1]"), None);
        assert!(is_json(&Value::Json("1".into())));
        assert!(is_json(&Value::Text("  {\"a\":1} ".into())));
        assert!(!is_json(&Value::Text("hello".into())));
    }
}
