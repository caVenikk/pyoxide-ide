use std::borrow::Cow;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use db_core::{Error, Result, Row, Value, ValueKind, convert};
use rusqlite::types::{ToSqlOutput, ValueRef};

use crate::error::map_error;

/// What a column's declared type says about its values, from the affinity
/// rules in <https://www.sqlite.org/datatype3.html> plus the common names
/// for booleans, dates, JSON and UUIDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decl {
    /// No declared type (an expression, or a column without a type).
    None,
    Bool,
    Int,
    Float,
    Decimal,
    Text,
    Bytes,
    Date,
    Time,
    Timestamp,
    TimestampTz,
    Json,
    Uuid,
}

impl Decl {
    pub(crate) fn classify(decl: Option<&str>) -> Decl {
        let Some(decl) = decl.map(str::trim).filter(|d| !d.is_empty()) else {
            return Decl::None;
        };
        let upper = decl.to_ascii_uppercase();
        let first = upper
            .split(|c: char| !c.is_ascii_alphanumeric())
            .find(|t| !t.is_empty())
            .unwrap_or("");
        match first {
            "BOOL" | "BOOLEAN" => return Decl::Bool,
            "JSON" | "JSONB" => return Decl::Json,
            "UUID" | "GUID" | "UNIQUEIDENTIFIER" => return Decl::Uuid,
            "DATE" => return Decl::Date,
            "TIME" | "TIMETZ" => return Decl::Time,
            "TIMESTAMPTZ" | "DATETIMEOFFSET" => return Decl::TimestampTz,
            "DATETIME" | "DATETIME2" | "SMALLDATETIME" | "TIMESTAMP" => {
                return if upper.contains("WITH TIME ZONE") {
                    Decl::TimestampTz
                } else {
                    Decl::Timestamp
                };
            }
            "DECIMAL" | "NUMERIC" | "NUMBER" | "MONEY" => return Decl::Decimal,
            _ => {}
        }
        // Affinity rules, in the documented order.
        if upper.contains("INT") {
            Decl::Int
        } else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
            Decl::Text
        } else if upper.contains("BLOB") {
            Decl::Bytes
        } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
            Decl::Float
        } else {
            // NUMERIC affinity for an unrecognised name: values keep their
            // stored class; text is the safest default for display.
            Decl::Text
        }
    }

    /// The column kind for a declared type; `None` when nothing is declared.
    pub(crate) fn kind(self) -> Option<ValueKind> {
        Some(match self {
            Decl::None => return None,
            Decl::Bool => ValueKind::Bool,
            Decl::Int => ValueKind::Int,
            Decl::Float => ValueKind::Float,
            Decl::Decimal => ValueKind::Decimal,
            Decl::Text => ValueKind::Text,
            Decl::Bytes => ValueKind::Bytes,
            Decl::Date => ValueKind::Date,
            Decl::Time => ValueKind::Time,
            Decl::Timestamp => ValueKind::Timestamp,
            Decl::TimestampTz => ValueKind::TimestampTz,
            Decl::Json => ValueKind::Json,
            Decl::Uuid => ValueKind::Uuid,
        })
    }
}

/// Storage class name and kind of a value, for columns without a declared type.
pub(crate) fn storage_class(v: &Value) -> (&'static str, ValueKind) {
    match v {
        Value::Null => ("NULL", ValueKind::Text),
        Value::Int(_) | Value::Bool(_) => ("INTEGER", ValueKind::Int),
        Value::Float(_) => ("REAL", ValueKind::Float),
        Value::Bytes(_) | Value::Uuid(_) => ("BLOB", ValueKind::Bytes),
        Value::Truncated { prefix, .. } => storage_class(prefix),
        _ => ("TEXT", ValueKind::Text),
    }
}

/// Converts SQLite cells to typed values using the declared column types.
pub(crate) struct Converter {
    decls: Vec<Decl>,
    max_bytes: usize,
}

impl Converter {
    pub(crate) fn new(decls: Vec<Decl>, max_bytes: usize) -> Self {
        Self { decls, max_bytes }
    }

    pub(crate) fn row(&self, row: &rusqlite::Row<'_>) -> Result<Row> {
        let mut out = Vec::with_capacity(self.decls.len());
        for (i, decl) in self.decls.iter().enumerate() {
            let cell = row.get_ref(i).map_err(|e| map_error(&e))?;
            out.push(self.value(*decl, cell));
        }
        Ok(out)
    }

    fn value(&self, decl: Decl, cell: ValueRef<'_>) -> Value {
        match cell {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(i) => match decl {
                Decl::Bool if i == 0 || i == 1 => Value::Bool(i == 1),
                _ => Value::Int(i),
            },
            ValueRef::Real(f) => Value::Float(f),
            ValueRef::Text(bytes) => self.text(decl, bytes),
            ValueRef::Blob(bytes) => self.blob(decl, bytes),
        }
    }

    fn text(&self, decl: Decl, bytes: &[u8]) -> Value {
        if bytes.len() > self.max_bytes {
            return convert::text_from_utf8(bytes, self.max_bytes);
        }
        let text = match std::str::from_utf8(bytes) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => String::from_utf8_lossy(bytes),
        };
        let typed = match decl {
            Decl::Date => parse_date(&text).map(Value::Date),
            Decl::Time => parse_time(&text).map(Value::Time),
            Decl::Timestamp | Decl::TimestampTz => parse_timestamp(&text),
            Decl::Json => is_json(&text).then(|| Value::Json(text.to_string())),
            Decl::Uuid => canonical_uuid_text(&text).map(Value::Uuid),
            _ => None,
        };
        typed.unwrap_or_else(|| Value::Text(text.into_owned()))
    }

    fn blob(&self, decl: Decl, bytes: &[u8]) -> Value {
        if bytes.len() > self.max_bytes {
            return convert::bytes_from_slice(bytes, self.max_bytes);
        }
        if decl == Decl::Uuid
            && let Ok(raw) = <[u8; 16]>::try_from(bytes)
        {
            return Value::Uuid(uuid_from_bytes(&raw));
        }
        Value::Bytes(bytes.to_vec())
    }
}

fn is_json(text: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok()
}

fn num(b: &[u8]) -> Option<u32> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    b.iter().try_fold(0u32, |acc, d| {
        acc.checked_mul(10)?.checked_add(u32::from(d - b'0'))
    })
}

/// `YYYY-MM-DD`.
pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    let b = s.trim().as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    NaiveDate::from_ymd_opt(
        i32::try_from(num(&b[0..4])?).ok()?,
        num(&b[5..7])?,
        num(&b[8..10])?,
    )
}

/// `HH:MM`, `HH:MM:SS` or `HH:MM:SS.fraction`.
pub(crate) fn parse_time(s: &str) -> Option<NaiveTime> {
    let b = s.trim().as_bytes();
    if b.len() < 5 || b[2] != b':' {
        return None;
    }
    let hour = num(&b[0..2])?;
    let minute = num(&b[3..5])?;
    let (second, nano) = match b.len() {
        5 => (0, 0),
        8 if b[5] == b':' => (num(&b[6..8])?, 0),
        n if n > 9 && b[5] == b':' && b[8] == b'.' => {
            let frac = &b[9..];
            if frac.len() > 9 {
                return None;
            }
            let digits = num(frac)?;
            let scale = 10u32.pow(9 - u32::try_from(frac.len()).ok()?);
            (num(&b[6..8])?, digits.checked_mul(scale)?)
        }
        _ => return None,
    };
    NaiveTime::from_hms_nano_opt(hour, minute, second, nano)
}

/// `YYYY-MM-DD[ T]HH:MM[:SS[.fraction]]` with an optional `Z` or `+HH:MM`
/// offset. Text with an offset gives `TimestampTz`, without one `Timestamp`.
pub(crate) fn parse_timestamp(s: &str) -> Option<Value> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 16 || !matches!(b[10], b' ' | b'T' | b't') {
        return None;
    }
    let date = parse_date(&s[..10])?;
    let rest = &s[11..];
    let split = rest.find(['Z', 'z', '+', '-']);
    let (time_text, offset_text) = match split {
        Some(i) => (rest[..i].trim_end(), Some(&rest[i..])),
        None => (rest, None),
    };
    let time = parse_time(time_text)?;
    let naive = NaiveDateTime::new(date, time);
    match offset_text {
        None => Some(Value::Timestamp(naive)),
        Some(o) => {
            let offset = parse_offset(o)?;
            let dt: DateTime<FixedOffset> = offset.from_local_datetime(&naive).single()?;
            Some(Value::TimestampTz(dt))
        }
    }
}

fn parse_offset(s: &str) -> Option<FixedOffset> {
    let b = s.as_bytes();
    if matches!(b, [b'Z'] | [b'z']) {
        return FixedOffset::east_opt(0);
    }
    let sign = match b.first()? {
        b'+' => 1i32,
        b'-' => -1,
        _ => return None,
    };
    let digits: Vec<u8> = b[1..].iter().copied().filter(|c| *c != b':').collect();
    let (h, m) = match digits.len() {
        2 => (num(&digits)?, 0),
        4 => (num(&digits[..2])?, num(&digits[2..])?),
        _ => return None,
    };
    if h > 23 || m > 59 {
        return None;
    }
    let secs = i32::try_from(h * 3600 + m * 60).ok()?;
    FixedOffset::east_opt(sign * secs)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Canonical lowercase form of a UUID given as 36 characters with dashes or
/// as 32 plain hex digits.
pub(crate) fn canonical_uuid_text(s: &str) -> Option<String> {
    let b = s.trim().as_bytes();
    let mut raw = [0u8; 16];
    let mut nibbles = 0usize;
    match b.len() {
        36 => {
            for (i, c) in b.iter().enumerate() {
                if matches!(i, 8 | 13 | 18 | 23) {
                    if *c != b'-' {
                        return None;
                    }
                    continue;
                }
                let v = hex_val(*c)?;
                raw[nibbles / 2] |= v << (4 * (1 - nibbles % 2));
                nibbles += 1;
            }
        }
        32 => {
            for c in b {
                let v = hex_val(*c)?;
                raw[nibbles / 2] |= v << (4 * (1 - nibbles % 2));
                nibbles += 1;
            }
        }
        _ => return None,
    }
    Some(uuid_from_bytes(&raw))
}

pub(crate) fn uuid_from_bytes(raw: &[u8; 16]) -> String {
    let mut out = String::with_capacity(36);
    for (i, byte) in raw.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A bind value as SQLite sees it.
pub(crate) fn bind_value(v: &Value) -> Result<ToSqlOutput<'_>> {
    use rusqlite::types::Value as Sql;
    Ok(match v {
        Value::Null => ToSqlOutput::Borrowed(ValueRef::Null),
        Value::Bool(b) => ToSqlOutput::Borrowed(ValueRef::Integer(i64::from(*b))),
        Value::Int(i) => ToSqlOutput::Borrowed(ValueRef::Integer(*i)),
        Value::Float(f) => ToSqlOutput::Borrowed(ValueRef::Real(*f)),
        Value::Decimal(s)
        | Value::Text(s)
        | Value::Interval(s)
        | Value::Uuid(s)
        | Value::Json(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
        Value::Other { text, .. } => ToSqlOutput::Borrowed(ValueRef::Text(text.as_bytes())),
        Value::Bytes(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b)),
        Value::Date(d) => ToSqlOutput::Owned(Sql::Text(d.format("%Y-%m-%d").to_string())),
        Value::Time(t) => ToSqlOutput::Owned(Sql::Text(t.format("%H:%M:%S%.f").to_string())),
        Value::Timestamp(t) => {
            ToSqlOutput::Owned(Sql::Text(t.format("%Y-%m-%d %H:%M:%S%.f").to_string()))
        }
        Value::TimestampTz(t) => {
            ToSqlOutput::Owned(Sql::Text(t.format("%Y-%m-%d %H:%M:%S%.f%:z").to_string()))
        }
        Value::Array(_) => {
            return Err(Error::unsupported(
                "SQLite has no array type; bind a JSON text value instead",
            ));
        }
        Value::Truncated { .. } => {
            return Err(Error::unsupported(
                "a truncated value cannot be bound; read the full value first",
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_types() {
        let c = Decl::classify;
        assert_eq!(c(None), Decl::None);
        assert_eq!(c(Some("")), Decl::None);
        assert_eq!(c(Some("INTEGER")), Decl::Int);
        assert_eq!(c(Some("unsigned big int")), Decl::Int);
        assert_eq!(c(Some("VARCHAR(255)")), Decl::Text);
        assert_eq!(c(Some("NVARCHAR(20)")), Decl::Text);
        assert_eq!(c(Some("CLOB")), Decl::Text);
        assert_eq!(c(Some("BLOB")), Decl::Bytes);
        assert_eq!(c(Some("DOUBLE PRECISION")), Decl::Float);
        assert_eq!(c(Some("FLOAT")), Decl::Float);
        assert_eq!(c(Some("DECIMAL(10,5)")), Decl::Decimal);
        assert_eq!(c(Some("NUMERIC")), Decl::Decimal);
        assert_eq!(c(Some("BOOLEAN")), Decl::Bool);
        assert_eq!(c(Some("bool")), Decl::Bool);
        assert_eq!(c(Some("DATE")), Decl::Date);
        assert_eq!(c(Some("TIME")), Decl::Time);
        assert_eq!(c(Some("DATETIME")), Decl::Timestamp);
        assert_eq!(c(Some("TIMESTAMP")), Decl::Timestamp);
        assert_eq!(c(Some("TIMESTAMP WITH TIME ZONE")), Decl::TimestampTz);
        assert_eq!(c(Some("TIMESTAMP WITHOUT TIME ZONE")), Decl::Timestamp);
        assert_eq!(c(Some("TIMESTAMPTZ")), Decl::TimestampTz);
        assert_eq!(c(Some("JSON")), Decl::Json);
        assert_eq!(c(Some("UUID")), Decl::Uuid);
        assert_eq!(c(Some("STRING")), Decl::Text);
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            parse_date("2026-01-02"),
            NaiveDate::from_ymd_opt(2026, 1, 2)
        );
        assert_eq!(parse_date("2026-13-02"), None);
        assert_eq!(parse_date("2026-1-2"), None);
        assert_eq!(parse_time("03:04"), NaiveTime::from_hms_opt(3, 4, 0));
        assert_eq!(
            parse_time("03:04:05.25"),
            NaiveTime::from_hms_milli_opt(3, 4, 5, 250)
        );
        assert_eq!(
            parse_time("03:04:05.123456"),
            NaiveTime::from_hms_micro_opt(3, 4, 5, 123_456)
        );
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("03:04:05.1234567890"), None);

        let naive = parse_timestamp("2026-01-02 03:04:05.5").unwrap();
        assert_eq!(naive.to_string(), "2026-01-02 03:04:05.500");
        assert!(matches!(naive, Value::Timestamp(_)));
        let t = parse_timestamp("2026-01-02T03:04").unwrap();
        assert!(matches!(t, Value::Timestamp(_)));
        let z = parse_timestamp("2026-01-02T03:04:05Z").unwrap();
        assert!(matches!(z, Value::TimestampTz(_)));
        let tz = parse_timestamp("2026-01-02 03:04:05+03:00").unwrap();
        assert_eq!(tz.to_string(), "2026-01-02 03:04:05 +03:00");
        let neg = parse_timestamp("2026-01-02 03:04:05-0530").unwrap();
        assert_eq!(neg.to_string(), "2026-01-02 03:04:05 -05:30");
        assert_eq!(parse_timestamp("yesterday"), None);
        assert_eq!(parse_timestamp("2026-01-02"), None);
    }

    #[test]
    fn uuids() {
        let raw = [
            0x12, 0x3e, 0x45, 0x67, 0xe8, 0x9b, 0x12, 0xd3, 0xa4, 0x56, 0x42, 0x66, 0x14, 0x17,
            0x40, 0x00,
        ];
        let text = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(uuid_from_bytes(&raw), text);
        assert_eq!(canonical_uuid_text(text).as_deref(), Some(text));
        assert_eq!(
            canonical_uuid_text(&text.to_uppercase()).as_deref(),
            Some(text)
        );
        assert_eq!(
            canonical_uuid_text("123e4567e89b12d3a456426614174000").as_deref(),
            Some(text)
        );
        assert_eq!(canonical_uuid_text("not-a-uuid"), None);
    }

    #[test]
    fn bind_forms() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        let ts = d.and_hms_micro_opt(3, 4, 5, 600).unwrap();
        let text = |v: &Value| match bind_value(v).unwrap() {
            ToSqlOutput::Owned(rusqlite::types::Value::Text(s)) => s,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(text(&Value::Date(d)), "2026-01-02");
        assert_eq!(text(&Value::Timestamp(ts)), "2026-01-02 03:04:05.000600");
        assert!(bind_value(&Value::Array(vec![])).is_err());
    }
}
