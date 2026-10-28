//! A field's text as a value of the column it is imported into.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use db_core::{Value, ValueKind};

const DATES: [&str; 3] = ["%Y-%m-%d", "%Y/%m/%d", "%d.%m.%Y"];
const TIMES: [&str; 3] = ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"];
const TIMESTAMPS: [&str; 6] = [
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M",
    "%Y-%m-%dT%H:%M",
    "%Y/%m/%d %H:%M:%S%.f",
    "%d.%m.%Y %H:%M:%S%.f",
];
const ZONED: [&str; 4] = [
    "%Y-%m-%d %H:%M:%S%.f %:z",
    "%Y-%m-%d %H:%M:%S%.f%:z",
    "%Y-%m-%dT%H:%M:%S%.f%:z",
    "%Y-%m-%d %H:%M:%S%.f %#z",
];

fn first<T>(text: &str, formats: &[&str], parse: impl Fn(&str, &str) -> Option<T>) -> Option<T> {
    formats.iter().find_map(|f| parse(text, f))
}

fn is_decimal(text: &str) -> bool {
    let t = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match t.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e.strip_prefix(['-', '+']).unwrap_or(e))),
        None => (t, None),
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mantissa_ok = match mantissa.split_once('.') {
        Some((a, b)) => {
            (a.is_empty() || digits(a))
                && (b.is_empty() || digits(b))
                && !(a.is_empty() && b.is_empty())
        }
        None => digits(mantissa),
    };
    mantissa_ok && exponent.is_none_or(digits)
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix("\\x"))?;
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn canonical_uuid(text: &str) -> Option<String> {
    let t = text.trim().trim_start_matches('{').trim_end_matches('}');
    let groups: Vec<&str> = t.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    (groups.len() == 5
        && groups
            .iter()
            .zip(lens)
            .all(|(g, l)| g.len() == l && g.chars().all(|c| c.is_ascii_hexdigit())))
    .then(|| t.to_ascii_lowercase())
}

/// The value of `text` for a column of `kind`; the error says what the
/// text should have been.
pub fn convert(text: &str, kind: ValueKind) -> Result<Value, String> {
    let t = text.trim();
    let bad = |what: &str| Err(format!("\"{}\" is not {what}", shorten(text)));
    match kind {
        ValueKind::Int => match t.parse::<i64>() {
            Ok(i) => Ok(Value::Int(i)),
            Err(_) => bad("an integer"),
        },
        ValueKind::Float => match t.parse::<f64>() {
            Ok(f) => Ok(Value::Float(f)),
            Err(_) => bad("a number"),
        },
        ValueKind::Decimal => {
            if is_decimal(t) {
                Ok(Value::Decimal(t.strip_prefix('+').unwrap_or(t).to_owned()))
            } else {
                bad("a number")
            }
        }
        ValueKind::Bool => match t.to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "1" => Ok(Value::Bool(true)),
            "false" | "f" | "no" | "n" | "0" => Ok(Value::Bool(false)),
            _ => bad("a boolean"),
        },
        ValueKind::Bytes => match hex_bytes(t) {
            Some(b) => Ok(Value::Bytes(b)),
            None => bad("binary data (0x followed by hex digits)"),
        },
        ValueKind::Date => match first(t, &DATES, |s, f| NaiveDate::parse_from_str(s, f).ok()) {
            Some(d) => Ok(Value::Date(d)),
            None => bad("a date"),
        },
        ValueKind::Time => match first(t, &TIMES, |s, f| NaiveTime::parse_from_str(s, f).ok()) {
            Some(d) => Ok(Value::Time(d)),
            None => bad("a time"),
        },
        ValueKind::Timestamp => {
            match first(t, &TIMESTAMPS, |s, f| {
                NaiveDateTime::parse_from_str(s, f).ok()
            })
            .or_else(|| {
                first(t, &DATES, |s, f| NaiveDate::parse_from_str(s, f).ok())
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
            }) {
                Some(d) => Ok(Value::Timestamp(d)),
                None => bad("a timestamp"),
            }
        }
        ValueKind::TimestampTz => {
            let zoned = first(t, &ZONED, |s, f| {
                DateTime::<FixedOffset>::parse_from_str(s, f).ok()
            })
            .or_else(|| DateTime::parse_from_rfc3339(t).ok());
            if let Some(d) = zoned {
                return Ok(Value::TimestampTz(d));
            }
            // Without a zone the time is taken as UTC.
            match first(t, &TIMESTAMPS, |s, f| {
                NaiveDateTime::parse_from_str(s, f).ok()
            }) {
                Some(n) => Ok(Value::TimestampTz(
                    DateTime::<FixedOffset>::from_naive_utc_and_offset(
                        n,
                        FixedOffset::east_opt(0).expect("zero is a valid offset"),
                    ),
                )),
                None => bad("a timestamp with a time zone"),
            }
        }
        ValueKind::Uuid => match canonical_uuid(t) {
            Some(u) => Ok(Value::Uuid(u)),
            None => bad("a UUID"),
        },
        ValueKind::Json => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(_) => Ok(Value::Json(text.to_owned())),
            Err(e) => Err(format!("not valid JSON: {e}")),
        },
        ValueKind::Interval => Ok(Value::Interval(text.to_owned())),
        ValueKind::Text | ValueKind::Array | ValueKind::Other => Ok(Value::Text(text.to_owned())),
    }
}

fn shorten(text: &str) -> String {
    if text.chars().count() > 40 {
        let cut: String = text.chars().take(39).collect();
        format!("{cut}…")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(convert(" 42 ", ValueKind::Int), Ok(Value::Int(42)));
        assert!(convert("4.2", ValueKind::Int).is_err());
        assert_eq!(
            convert("10.50", ValueKind::Decimal),
            Ok(Value::Decimal("10.50".into()))
        );
        assert_eq!(
            convert("+1e5", ValueKind::Decimal),
            Ok(Value::Decimal("1e5".into()))
        );
        assert!(convert("1,5", ValueKind::Decimal).is_err());
        assert!(convert("abc", ValueKind::Decimal).is_err());
        assert_eq!(convert("2.5", ValueKind::Float), Ok(Value::Float(2.5)));
    }

    #[test]
    fn booleans_binary_and_uuid() {
        assert_eq!(convert("Yes", ValueKind::Bool), Ok(Value::Bool(true)));
        assert_eq!(convert("0", ValueKind::Bool), Ok(Value::Bool(false)));
        assert!(convert("maybe", ValueKind::Bool).is_err());
        assert_eq!(
            convert("0xCAFE", ValueKind::Bytes),
            Ok(Value::Bytes(vec![0xca, 0xfe]))
        );
        assert_eq!(
            convert("\\xca", ValueKind::Bytes),
            Ok(Value::Bytes(vec![0xca]))
        );
        assert!(convert("0xCAF", ValueKind::Bytes).is_err());
        assert_eq!(
            convert("A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11", ValueKind::Uuid),
            Ok(Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()))
        );
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            convert("2026-09-01", ValueKind::Date),
            Ok(Value::Date(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()))
        );
        assert_eq!(
            convert("01.09.2026", ValueKind::Date),
            Ok(Value::Date(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()))
        );
        assert!(convert("2026-13-01", ValueKind::Date).is_err());
        assert!(matches!(
            convert("2026-09-01 03:27:00.123", ValueKind::Timestamp),
            Ok(Value::Timestamp(_))
        ));
        assert!(matches!(
            convert("2026-09-01T03:27:00Z", ValueKind::TimestampTz),
            Ok(Value::TimestampTz(_))
        ));
        let Ok(Value::TimestampTz(t)) =
            convert("2026-09-01 03:27:00.000000 +02:00", ValueKind::TimestampTz)
        else {
            panic!("not parsed");
        };
        assert_eq!(t.to_rfc3339(), "2026-09-01T03:27:00+02:00");
        assert!(matches!(
            convert("03:27", ValueKind::Time),
            Ok(Value::Time(_))
        ));
    }

    #[test]
    fn json_and_text() {
        assert_eq!(
            convert("{\"a\": 1}", ValueKind::Json),
            Ok(Value::Json("{\"a\": 1}".into()))
        );
        assert!(convert("{nope", ValueKind::Json).is_err());
        assert_eq!(
            convert(" keeps spaces ", ValueKind::Text),
            Ok(Value::Text(" keeps spaces ".into()))
        );
    }

    #[test]
    fn the_error_names_the_text() {
        let e = convert("abc", ValueKind::Int).unwrap_err();
        assert_eq!(e, "\"abc\" is not an integer");
    }
}
