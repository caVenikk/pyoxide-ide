//! Column metadata and cell values from oracle-rs to the db-core model.
//!
//! oracle-rs 0.1.7 decodes NUMBER (as text), character data, RAW, DATE,
//! TIMESTAMP (to microseconds), LOB locators, JSON (21c+) and VECTOR (23ai).
//! Everything it gets wrong is converted on the server by the wrapper block
//! (see `plsql.rs`) and arrives here as text or RAW with a tag.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use db_core::{Column, Value, ValueKind};
use oracle_rs::{ColumnInfo, LobLocator, LobValue, OracleType, OracleVector, VectorData};

use crate::plsql::split_tag;

/// A server-side conversion applied by the wrapper block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    NVarchar2,
    NChar,
    /// A column whose type has no length (`NULL`, `''`): always NULL.
    Null,
    BinaryFloat,
    BinaryDouble,
    IntervalYm,
    IntervalDs,
    TimestampTz,
    Timestamp,
    TimestampLtz,
    Rowid,
    Urowid,
    Nclob,
    XmlType,
    Bfile,
    Object,
}

impl Tag {
    pub fn parse(s: &str) -> Option<Tag> {
        Some(match s {
            "NVARCHAR2" => Tag::NVarchar2,
            "NCHAR" => Tag::NChar,
            "NULL" => Tag::Null,
            "BINARY_FLOAT" => Tag::BinaryFloat,
            "BINARY_DOUBLE" => Tag::BinaryDouble,
            "INTERVAL_YM" => Tag::IntervalYm,
            "INTERVAL_DS" => Tag::IntervalDs,
            "TSTZ" => Tag::TimestampTz,
            "TS" => Tag::Timestamp,
            "TSLTZ" => Tag::TimestampLtz,
            "ROWID" => Tag::Rowid,
            "UROWID" => Tag::Urowid,
            "NCLOB" => Tag::Nclob,
            "XMLTYPE" => Tag::XmlType,
            "BFILE" => Tag::Bfile,
            "OBJECT" => Tag::Object,
            _ => return None,
        })
    }

    fn type_name(self) -> &'static str {
        match self {
            Tag::NVarchar2 => "NVARCHAR2",
            Tag::NChar => "NCHAR",
            Tag::Null => "VARCHAR2",
            Tag::BinaryFloat => "BINARY_FLOAT",
            Tag::BinaryDouble => "BINARY_DOUBLE",
            Tag::IntervalYm => "INTERVAL YEAR TO MONTH",
            Tag::IntervalDs => "INTERVAL DAY TO SECOND",
            Tag::TimestampTz => "TIMESTAMP WITH TIME ZONE",
            Tag::Timestamp => "TIMESTAMP",
            Tag::TimestampLtz => "TIMESTAMP WITH LOCAL TIME ZONE",
            Tag::Rowid => "ROWID",
            Tag::Urowid => "UROWID",
            Tag::Nclob => "NCLOB",
            Tag::XmlType => "XMLTYPE",
            Tag::Bfile => "BFILE",
            Tag::Object => "OBJECT",
        }
    }

    fn kind(self) -> ValueKind {
        match self {
            Tag::NVarchar2 | Tag::NChar | Tag::Null | Tag::Nclob | Tag::XmlType => ValueKind::Text,
            Tag::BinaryFloat | Tag::BinaryDouble => ValueKind::Float,
            Tag::IntervalYm | Tag::IntervalDs => ValueKind::Interval,
            Tag::TimestampTz => ValueKind::TimestampTz,
            Tag::Timestamp | Tag::TimestampLtz => ValueKind::Timestamp,
            Tag::Rowid | Tag::Urowid | Tag::Bfile | Tag::Object => ValueKind::Other,
        }
    }
}

/// How NUMBER values of a column map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberShape {
    /// `NUMBER(p)` / `NUMBER(p,0)`: integers.
    Integer,
    /// `NUMBER(p,s)` with `s > 0`: always exact decimals.
    Scaled,
    /// `NUMBER`, `FLOAT`, expressions: integers when they fit, else decimals.
    Free,
}

/// Per-column decoding plan.
#[derive(Debug, Clone)]
pub struct ColPlan {
    pub oracle_type: OracleType,
    pub tag: Option<Tag>,
    number: NumberShape,
    /// Declared maximum size from the describe data, for fetch sizing.
    pub declared_bytes: usize,
}

fn signed_scale(info: &ColumnInfo) -> i16 {
    // oracle-rs keeps the SB1 scale as an unsigned byte (129 = -127).
    (info.scale as u8) as i8 as i16
}

fn base_type_name(t: OracleType) -> &'static str {
    match t {
        OracleType::Varchar => "VARCHAR2",
        OracleType::Number => "NUMBER",
        OracleType::BinaryInteger => "BINARY_INTEGER",
        OracleType::Long => "LONG",
        OracleType::Rowid => "ROWID",
        OracleType::Date => "DATE",
        OracleType::Raw => "RAW",
        OracleType::LongRaw => "LONG RAW",
        OracleType::Char => "CHAR",
        OracleType::BinaryFloat => "BINARY_FLOAT",
        OracleType::BinaryDouble => "BINARY_DOUBLE",
        OracleType::Cursor => "REF CURSOR",
        OracleType::Object => "OBJECT",
        OracleType::Clob => "CLOB",
        OracleType::Blob => "BLOB",
        OracleType::Bfile => "BFILE",
        OracleType::Json => "JSON",
        OracleType::Vector => "VECTOR",
        OracleType::Timestamp => "TIMESTAMP",
        OracleType::TimestampTz => "TIMESTAMP WITH TIME ZONE",
        OracleType::IntervalYm => "INTERVAL YEAR TO MONTH",
        OracleType::IntervalDs => "INTERVAL DAY TO SECOND",
        OracleType::Urowid => "UROWID",
        OracleType::TimestampLtz => "TIMESTAMP WITH LOCAL TIME ZONE",
        OracleType::Boolean => "BOOLEAN",
    }
}

fn base_kind(t: OracleType, number: NumberShape) -> ValueKind {
    match t {
        OracleType::Number => {
            if number == NumberShape::Integer {
                ValueKind::Int
            } else {
                ValueKind::Decimal
            }
        }
        OracleType::BinaryInteger => ValueKind::Int,
        OracleType::BinaryFloat | OracleType::BinaryDouble => ValueKind::Float,
        OracleType::Varchar | OracleType::Char | OracleType::Long | OracleType::Clob => {
            ValueKind::Text
        }
        OracleType::Raw | OracleType::LongRaw | OracleType::Blob => ValueKind::Bytes,
        OracleType::Date | OracleType::Timestamp | OracleType::TimestampLtz => {
            ValueKind::Timestamp
        }
        OracleType::TimestampTz => ValueKind::TimestampTz,
        OracleType::IntervalYm | OracleType::IntervalDs => ValueKind::Interval,
        OracleType::Json => ValueKind::Json,
        OracleType::Boolean => ValueKind::Bool,
        _ => ValueKind::Other,
    }
}

fn declared_bytes(info: &ColumnInfo) -> usize {
    let size = info.data_size as usize;
    match info.oracle_type {
        OracleType::Number | OracleType::BinaryInteger => 22,
        OracleType::BinaryFloat => 5,
        OracleType::BinaryDouble => 9,
        OracleType::Date => 8,
        OracleType::Timestamp | OracleType::TimestampLtz => 12,
        OracleType::TimestampTz => 14,
        OracleType::IntervalYm => 6,
        OracleType::IntervalDs => 12,
        OracleType::Boolean => 2,
        OracleType::Clob
        | OracleType::Blob
        | OracleType::Bfile
        | OracleType::Json
        | OracleType::Vector => 160,
        OracleType::Long | OracleType::LongRaw => 32_768,
        OracleType::Rowid | OracleType::Urowid => 64,
        _ => size.clamp(1, 32_767) + 3,
    }
}

/// Column metadata and decoding plan for one described column.
pub fn column(info: &ColumnInfo) -> (Column, ColPlan) {
    let (name, tag) = split_tag(&info.name);
    let tag = tag.and_then(Tag::parse);
    let scale = signed_scale(info);
    let number = if info.precision == 0 {
        NumberShape::Free
    } else if scale > 0 {
        NumberShape::Scaled
    } else if scale == 0 && info.precision <= 18 {
        NumberShape::Integer
    } else {
        NumberShape::Free
    };
    let (type_name, kind) = match tag {
        Some(t) => (t.type_name().to_owned(), t.kind()),
        None => {
            let base = base_type_name(info.oracle_type);
            let type_name = if info.oracle_type == OracleType::Number
                && info.precision > 0
                && scale == -127
            {
                "FLOAT".to_owned()
            } else {
                base.to_owned()
            };
            (type_name, base_kind(info.oracle_type, number))
        }
    };
    (
        Column {
            name: name.to_owned(),
            type_name,
            kind,
            nullable: info.nullable,
        },
        ColPlan {
            oracle_type: info.oracle_type,
            tag,
            number,
            declared_bytes: declared_bytes(info),
        },
    )
}

/// A decoded cell, or a LOB that still has to be read.
pub enum Cell {
    Ready(Value),
    Lob(LobLocator),
}

/// Cut text to `max` bytes on a character boundary.
pub fn truncate_text(s: String, max: usize, full_len: Option<u64>) -> Value {
    if s.len() <= max {
        return match full_len {
            Some(n) if n > s.chars().count() as u64 => Value::Truncated {
                prefix: Box::new(Value::Text(s)),
                full_len: Some(n),
            },
            _ => Value::Text(s),
        };
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let full = full_len.unwrap_or(s.len() as u64);
    Value::Truncated {
        prefix: Box::new(Value::Text(s[..cut].to_owned())),
        full_len: Some(full),
    }
}

/// Cut bytes to `max`.
pub fn truncate_bytes(mut b: Vec<u8>, max: usize, full_len: Option<u64>) -> Value {
    let full = full_len.unwrap_or(b.len() as u64);
    if b.len() <= max && full <= b.len() as u64 {
        return Value::Bytes(b);
    }
    b.truncate(max);
    Value::Truncated {
        prefix: Box::new(Value::Bytes(b)),
        full_len: Some(full),
    }
}

/// NUMBER text (`-12.5`, `12345678901234567890`) to Int or Decimal.
fn number_value(text: &str, shape: NumberShape) -> Value {
    let integral = !text.contains(['.', 'e', 'E']);
    if shape != NumberShape::Scaled
        && integral
        && let Ok(i) = text.parse::<i64>()
    {
        return Value::Int(i);
    }
    Value::Decimal(text.to_owned())
}

fn parse_date_part(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s.trim_start_matches('+')),
    };
    let mut it = body.splitn(3, '-');
    let y: i32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    NaiveDate::from_ymd_opt(if neg { -y } else { y }, m, d)
}

fn parse_time_part(s: &str) -> Option<NaiveTime> {
    let (hms, frac) = s.split_once('.').unwrap_or((s, "0"));
    let mut it = hms.splitn(3, ':');
    let h: u32 = it.next()?.parse().ok()?;
    let mi: u32 = it.next()?.parse().ok()?;
    let sec: u32 = it.next()?.parse().ok()?;
    let digits: String = frac.chars().take(9).collect();
    let nanos: u32 = format!("{digits:0<9}").parse().ok()?;
    NaiveTime::from_hms_nano_opt(h, mi, sec, nanos)
}

/// `SYYYY-MM-DD HH24:MI:SS.FF9` (leading sign or space for the year).
pub fn parse_timestamp_text(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    let (date, time) = s.rsplit_once(' ')?;
    Some(NaiveDateTime::new(parse_date_part(date)?, parse_time_part(time)?))
}

/// `SYYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM`.
pub fn parse_timestamp_tz_text(s: &str) -> Option<DateTime<FixedOffset>> {
    let s = s.trim();
    let (rest, tz) = s.rsplit_once(' ')?;
    let naive = parse_timestamp_text(rest)?;
    let (sign, hm) = match tz.strip_prefix('-') {
        Some(t) => (-1, t),
        None => (1, tz.trim_start_matches('+')),
    };
    let (h, m) = hm.split_once(':')?;
    let secs = sign * (h.parse::<i32>().ok()? * 3600 + m.parse::<i32>().ok()? * 60);
    let offset = FixedOffset::east_opt(secs)?;
    offset.from_local_datetime(&naive).single()
}

fn other(type_name: &str, text: impl Into<String>) -> Value {
    Value::Other {
        type_name: type_name.to_owned(),
        text: text.into(),
    }
}

fn vector_text(v: &OracleVector) -> String {
    fn join<T: ToString>(items: &[T]) -> String {
        let parts: Vec<String> = items.iter().map(ToString::to_string).collect();
        format!("[{}]", parts.join(","))
    }
    match v {
        OracleVector::Dense(VectorData::Float32(x)) => join(x),
        OracleVector::Dense(VectorData::Float64(x)) => join(x),
        OracleVector::Dense(VectorData::Int8(x)) => join(x),
        OracleVector::Dense(VectorData::Binary(x)) => join(x),
        OracleVector::Sparse(s) => format!("{s:?}"),
    }
}

fn naive_from_parts(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    micros: u32,
) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(year, month.into(), day.into())?.and_hms_micro_opt(
        hour.into(),
        minute.into(),
        second.into(),
        micros,
    )
}

/// Decode one cell. `max` is `ExecOptions::max_value_bytes`.
pub fn cell(plan: &ColPlan, v: oracle_rs::Value, max: usize) -> Cell {
    use oracle_rs::Value as O;
    let ready = |v: Value| Cell::Ready(v);
    if let Some(tag) = plan.tag {
        return ready(match (tag, v) {
            (_, O::Null) => Value::Null,
            (Tag::Null, _) => Value::Null,
            (Tag::BinaryDouble, O::Bytes(b)) if b.len() == 8 => {
                Value::Float(f64::from_be_bytes(b.try_into().unwrap_or([0; 8])))
            }
            (Tag::BinaryFloat, O::Bytes(b)) if b.len() == 4 => Value::Float(f64::from(
                f32::from_be_bytes(b.try_into().unwrap_or([0; 4])),
            )),
            (Tag::IntervalYm | Tag::IntervalDs, O::String(s)) => Value::Interval(s),
            (Tag::TimestampTz, O::String(s)) => match parse_timestamp_tz_text(&s) {
                Some(t) => Value::TimestampTz(t),
                None => other("TIMESTAMP WITH TIME ZONE", s),
            },
            (Tag::Timestamp | Tag::TimestampLtz, O::String(s)) => {
                match parse_timestamp_text(&s) {
                    Some(t) => Value::Timestamp(t),
                    None => other(tag.type_name(), s),
                }
            }
            (Tag::Rowid | Tag::Urowid | Tag::Bfile, O::String(s)) => other(tag.type_name(), s),
            (Tag::Object, O::String(s)) => {
                let name = s.trim_start_matches('(').trim_end_matches(')').to_owned();
                other(&name, s)
            }
            (Tag::NVarchar2 | Tag::NChar, O::String(s)) => truncate_text(s, max, None),
            (Tag::Nclob | Tag::XmlType, O::Lob(lob)) => return lob_cell(plan, lob, max),
            (Tag::Nclob | Tag::XmlType, O::String(s)) => truncate_text(s, max, None),
            (t, other_value) => other(t.type_name(), format!("{other_value:?}")),
        });
    }
    ready(match v {
        O::Null => Value::Null,
        O::String(s) => match plan.oracle_type {
            OracleType::Number | OracleType::BinaryInteger => number_value(&s, plan.number),
            OracleType::Boolean => Value::Bool(s.as_bytes().last() == Some(&1)),
            OracleType::Rowid | OracleType::Urowid => other("ROWID", s),
            OracleType::BinaryFloat
            | OracleType::BinaryDouble
            | OracleType::IntervalYm
            | OracleType::IntervalDs => other(base_type_name(plan.oracle_type), s),
            _ => truncate_text(s, max, None),
        },
        O::Integer(i) => Value::Int(i),
        O::Float(x) => Value::Float(x),
        O::Number(n) => number_value(n.as_str(), plan.number),
        O::Bytes(b) => truncate_bytes(b, max, None),
        O::Boolean(b) => Value::Bool(b),
        O::Date(d) => match naive_from_parts(d.year, d.month, d.day, d.hour, d.minute, d.second, 0)
        {
            Some(t) => Value::Timestamp(t),
            None => other("DATE", format!("{d:?}")),
        },
        O::Timestamp(ts) => {
            let naive = naive_from_parts(
                ts.year,
                ts.month,
                ts.day,
                ts.hour,
                ts.minute,
                ts.second,
                ts.microsecond,
            );
            match (naive, plan.oracle_type) {
                (Some(n), OracleType::TimestampTz) => {
                    // The wire value is UTC plus the offset of the value.
                    let secs = i32::from(ts.tz_hour_offset) * 3600
                        + i32::from(ts.tz_minute_offset) * 60;
                    match FixedOffset::east_opt(secs) {
                        Some(off) => Value::TimestampTz(off.from_utc_datetime(&n)),
                        None => Value::Timestamp(n),
                    }
                }
                (Some(n), _) => Value::Timestamp(n),
                (None, _) => other("TIMESTAMP", format!("{ts:?}")),
            }
        }
        O::RowId(r) => other("ROWID", r.to_string().unwrap_or_default()),
        O::Lob(lob) => return lob_cell(plan, lob, max),
        O::Json(j) => {
            if j.is_null() {
                Value::Null
            } else {
                Value::Json(j.to_string())
            }
        }
        O::Vector(v) => other("VECTOR", vector_text(&v)),
        O::Cursor(_) => other("REF CURSOR", "(cursor)"),
        O::Collection(c) => other("OBJECT", format!("({})", c.type_name)),
    })
}

fn lob_cell(plan: &ColPlan, lob: LobValue, max: usize) -> Cell {
    let is_blob = plan.oracle_type == OracleType::Blob;
    match lob {
        LobValue::Null => Cell::Ready(Value::Null),
        LobValue::Empty => Cell::Ready(if is_blob {
            Value::Bytes(Vec::new())
        } else {
            Value::Text(String::new())
        }),
        LobValue::Inline(b) => Cell::Ready(if is_blob {
            truncate_bytes(b.to_vec(), max, None)
        } else {
            truncate_text(String::from_utf8_lossy(&b).into_owned(), max, None)
        }),
        LobValue::Locator(loc) => Cell::Lob(loc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, t: OracleType, precision: i16, scale: i16, size: u32) -> ColumnInfo {
        let mut c = ColumnInfo::new(name, t);
        c.precision = precision;
        c.scale = scale;
        c.data_size = size;
        c
    }

    fn ready(c: Cell) -> Value {
        match c {
            Cell::Ready(v) => v,
            Cell::Lob(_) => panic!("unexpected LOB"),
        }
    }

    #[test]
    fn number_columns() {
        let (col, plan) = column(&info("ID", OracleType::Number, 10, 0, 22));
        assert_eq!(col.kind, ValueKind::Int);
        assert_eq!(col.type_name, "NUMBER");
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("42".into()), 100)), Value::Int(42));
        assert_eq!(
            ready(cell(&plan, oracle_rs::Value::String("12345678901234567890".into()), 100)),
            Value::Decimal("12345678901234567890".into())
        );

        let (col, plan) = column(&info("AMOUNT", OracleType::Number, 12, 2, 22));
        assert_eq!(col.kind, ValueKind::Decimal);
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("12".into()), 100)), Value::Decimal("12".into()));
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("-0.25".into()), 100)), Value::Decimal("-0.25".into()));

        // Expressions: precision 0, scale -127 stored as 129.
        let (col, plan) = column(&info("X", OracleType::Number, 0, 129, 22));
        assert_eq!(col.kind, ValueKind::Decimal);
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("42".into()), 100)), Value::Int(42));
        assert_eq!(
            ready(cell(&plan, oracle_rs::Value::String("12345678901234567890.0123456789".into()), 100)),
            Value::Decimal("12345678901234567890.0123456789".into())
        );

        let (col, _) = column(&info("F", OracleType::Number, 126, 129, 22));
        assert_eq!(col.type_name, "FLOAT");
        let (col, _) = column(&info("BIG", OracleType::Number, 38, 0, 22));
        assert_eq!(col.kind, ValueKind::Decimal);
    }

    #[test]
    fn tagged_columns() {
        let (col, plan) = column(&info("BD#PXT=BINARY_DOUBLE", OracleType::Raw, 0, 0, 8));
        assert_eq!(col.name, "BD");
        assert_eq!(col.type_name, "BINARY_DOUBLE");
        assert_eq!(col.kind, ValueKind::Float);
        let v = ready(cell(&plan, oracle_rs::Value::Bytes(1.5f64.to_be_bytes().to_vec()), 100));
        assert_eq!(v, Value::Float(1.5));

        let (_, plan) = column(&info("BF#PXT=BINARY_FLOAT", OracleType::Raw, 0, 0, 4));
        let v = ready(cell(&plan, oracle_rs::Value::Bytes(2.25f32.to_be_bytes().to_vec()), 100));
        assert_eq!(v, Value::Float(2.25));

        let (col, plan) = column(&info("I#PXT=INTERVAL_YM", OracleType::Varchar, 0, 0, 6));
        assert_eq!(col.kind, ValueKind::Interval);
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("+01-02".into()), 100)), Value::Interval("+01-02".into()));

        let (col, plan) = column(&info("T#PXT=TSTZ", OracleType::Varchar, 0, 0, 40));
        assert_eq!(col.kind, ValueKind::TimestampTz);
        let v = ready(cell(&plan, oracle_rs::Value::String(" 2026-01-02 03:04:05.500000000 +03:00".into()), 100));
        assert_eq!(v, Value::TimestampTz(DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5+03:00").unwrap()));

        let (_, plan) = column(&info("T#PXT=TS", OracleType::Varchar, 0, 0, 40));
        let v = ready(cell(&plan, oracle_rs::Value::String("2026-01-02 03:04:05.123456789".into()), 100));
        assert_eq!(v.to_string(), "2026-01-02 03:04:05.123456789");

        let (col, plan) = column(&info("R#PXT=ROWID", OracleType::Varchar, 0, 0, 18));
        assert_eq!(col.kind, ValueKind::Other);
        assert_eq!(
            ready(cell(&plan, oracle_rs::Value::String("AAAA".into()), 100)),
            Value::Other { type_name: "ROWID".into(), text: "AAAA".into() }
        );

        let (_, plan) = column(&info("N#PXT=NULL", OracleType::Varchar, 0, 0, 1));
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("x".into()), 100)), Value::Null);
    }

    #[test]
    fn timestamps_and_dates() {
        let (_, plan) = column(&info("TZ", OracleType::TimestampTz, 0, 6, 13));
        let ts = oracle_rs::types::OracleTimestamp::with_timezone(2026, 1, 2, 0, 4, 5, 500_000, 3, 0);
        // Wire value is UTC 00:04:05 with +03:00: local 03:04:05.
        let v = ready(cell(&plan, oracle_rs::Value::Timestamp(ts), 100));
        assert_eq!(v.to_string(), "2026-01-02 03:04:05.500 +03:00");

        let (col, plan) = column(&info("D", OracleType::Date, 0, 0, 7));
        assert_eq!(col.kind, ValueKind::Timestamp);
        let d = oracle_rs::types::OracleDate::new(-4000, 1, 1, 0, 0, 0);
        let v = ready(cell(&plan, oracle_rs::Value::Date(d), 100));
        assert!(matches!(v, Value::Timestamp(t) if t.date() == NaiveDate::from_ymd_opt(-4000, 1, 1).unwrap()));

        assert_eq!(
            parse_timestamp_text("-4712-01-01 00:00:00.000000000").unwrap().date(),
            NaiveDate::from_ymd_opt(-4712, 1, 1).unwrap()
        );
        assert!(parse_timestamp_tz_text("2026-01-02 03:04:05.5 -08:00").is_some());
        assert!(parse_timestamp_tz_text("garbage").is_none());
    }

    #[test]
    fn booleans_text_and_truncation() {
        let (col, plan) = column(&info("B", OracleType::Boolean, 0, 0, 1));
        assert_eq!(col.kind, ValueKind::Bool);
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("\u{1}\u{1}".into()), 10)), Value::Bool(true));
        assert_eq!(ready(cell(&plan, oracle_rs::Value::String("\0".into()), 10)), Value::Bool(false));

        let (_, plan) = column(&info("S", OracleType::Varchar, 0, 0, 100));
        let v = ready(cell(&plan, oracle_rs::Value::String("héllo".into()), 2));
        assert_eq!(
            v,
            Value::Truncated { prefix: Box::new(Value::Text("h".into())), full_len: Some(6) }
        );
        assert_eq!(
            truncate_bytes(vec![1, 2, 3], 2, Some(10)),
            Value::Truncated { prefix: Box::new(Value::Bytes(vec![1, 2])), full_len: Some(10) }
        );
        assert_eq!(truncate_bytes(vec![1, 2], 2, Some(2)), Value::Bytes(vec![1, 2]));
    }
}
