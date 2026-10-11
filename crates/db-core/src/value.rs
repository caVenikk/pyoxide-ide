use std::fmt::{self, Write as _};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

/// A typed cell value, the same for every backend.
///
/// Exact numbers that do not fit `i64` stay as text in [`Value::Decimal`] so
/// nothing is rounded on the way to the grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Exact decimal in plain notation (`-12.50`, `12345678901234567890`).
    Decimal(String),
    Text(String),
    #[serde(with = "hex_bytes")]
    Bytes(Vec<u8>),
    Date(NaiveDate),
    Time(NaiveTime),
    Timestamp(NaiveDateTime),
    TimestampTz(DateTime<FixedOffset>),
    /// Interval in the backend's canonical text form.
    Interval(String),
    /// Canonical lowercase form `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    Uuid(String),
    /// JSON document as text, exactly as the server returned it.
    Json(String),
    Array(Vec<Value>),
    /// A value cut to `ExecOptions::max_value_bytes` (the prefix is at most
    /// that many bytes, cut on a character boundary for text). `full_len` is
    /// the full length when the backend reports it, in the backend's own
    /// length unit: bytes in general, but for Oracle CLOB and NCLOB values
    /// it can be the LOB length in characters (UTF-16 code units, as Oracle
    /// counts them), which is what the server reports without reading the
    /// whole value.
    Truncated {
        prefix: Box<Value>,
        full_len: Option<u64>,
    },
    /// Anything without a native mapping: the backend type name and a text form.
    Other {
        type_name: String,
        text: String,
    },
}

/// The shape of a column's values, for alignment and editors in the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
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
    Interval,
    Uuid,
    Json,
    Array,
    Other,
}

impl ValueKind {
    pub fn is_numeric(self) -> bool {
        matches!(self, ValueKind::Int | ValueKind::Float | ValueKind::Decimal)
    }
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The kind of this value; `None` for `Null`.
    pub fn kind(&self) -> Option<ValueKind> {
        Some(match self {
            Value::Null => return None,
            Value::Bool(_) => ValueKind::Bool,
            Value::Int(_) => ValueKind::Int,
            Value::Float(_) => ValueKind::Float,
            Value::Decimal(_) => ValueKind::Decimal,
            Value::Text(_) => ValueKind::Text,
            Value::Bytes(_) => ValueKind::Bytes,
            Value::Date(_) => ValueKind::Date,
            Value::Time(_) => ValueKind::Time,
            Value::Timestamp(_) => ValueKind::Timestamp,
            Value::TimestampTz(_) => ValueKind::TimestampTz,
            Value::Interval(_) => ValueKind::Interval,
            Value::Uuid(_) => ValueKind::Uuid,
            Value::Json(_) => ValueKind::Json,
            Value::Array(_) => ValueKind::Array,
            Value::Truncated { prefix, .. } => return prefix.kind(),
            Value::Other { .. } => ValueKind::Other,
        })
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) | Value::Decimal(s) | Value::Json(s) | Value::Uuid(s) => Some(s),
            Value::Other { text, .. } => Some(text),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Decimal(s) => s.parse().ok(),
            Value::Bool(b) => Some(i64::from(*b)),
            _ => None,
        }
    }

    /// Text form for display, `None` for SQL NULL.
    pub fn to_text(&self) -> Option<String> {
        (!self.is_null()).then(|| self.to_string())
    }
}

/// Display form used by the CLI table and as the default grid text.
/// `Null` prints as `NULL`; bytes print as `0x` + hex.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("NULL"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Decimal(s)
            | Value::Text(s)
            | Value::Interval(s)
            | Value::Uuid(s)
            | Value::Json(s) => f.write_str(s),
            Value::Bytes(b) => {
                f.write_str("0x")?;
                for byte in b {
                    write!(f, "{byte:02X}")?;
                }
                Ok(())
            }
            Value::Date(d) => write!(f, "{}", d.format("%Y-%m-%d")),
            Value::Time(t) => write!(f, "{}", t.format("%H:%M:%S%.f")),
            Value::Timestamp(t) => write!(f, "{}", t.format("%Y-%m-%d %H:%M:%S%.f")),
            Value::TimestampTz(t) => write!(f, "{}", t.format("%Y-%m-%d %H:%M:%S%.f %:z")),
            Value::Array(items) => {
                f.write_char('{')?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_char(',')?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_char('}')
            }
            Value::Truncated { prefix, .. } => write!(f, "{prefix}…"),
            Value::Other { text, .. } => f.write_str(text),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<f64> for Value {
    fn from(x: f64) -> Self {
        Value::Float(x)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

/// Bytes travel as hex strings in JSON (the command channel), not as arrays.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() % 2 != 0 {
            return Err(D::Error::custom("odd hex length"));
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(D::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_forms() {
        assert_eq!(Value::Null.to_string(), "NULL");
        assert_eq!(Value::Bytes(vec![0xde, 0xad]).to_string(), "0xDEAD");
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(Value::Date(d).to_string(), "2026-01-02");
        let ts = d.and_hms_micro_opt(3, 4, 5, 600).unwrap();
        assert_eq!(
            Value::Timestamp(ts).to_string(),
            "2026-01-02 03:04:05.000600"
        );
        let arr = Value::Array(vec![Value::Int(1), Value::Null]);
        assert_eq!(arr.to_string(), "{1,NULL}");
        let cut = Value::Truncated {
            prefix: Box::new(Value::Text("abc".into())),
            full_len: Some(10),
        };
        assert_eq!(cut.to_string(), "abc…");
        assert_eq!(cut.kind(), Some(ValueKind::Text));
    }

    #[test]
    fn json_round_trip() {
        let vals = vec![
            Value::Null,
            Value::Decimal("123456789012345678901234567890.5".into()),
            Value::Bytes(vec![0, 255]),
            Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05+03:00").unwrap()),
        ];
        let json = serde_json::to_string(&vals).unwrap();
        assert!(json.contains("\"00ff\""));
        let back: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, vals);
    }
}
