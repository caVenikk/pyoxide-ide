//! Oracle types to `db_core` values.
//!
//! Everything that can be decided without a session is a pure function here
//! and unit-tested; only `convert_cell` touches a fetched value.

use std::cell::Cell;
use std::io::Read;

use chrono::{FixedOffset, NaiveDate, TimeZone};
use db_core::{Column, Error, Result, Value, ValueKind};
use oracle::sql_type::{
    Blob, Clob, InnerValue, IntervalDS, IntervalYM, Lob, Nclob, OracleType, Timestamp,
};

use crate::errors;

/// A CLOB is read in one call when its length is at most this many times
/// `max_value_bytes` UTF-16 units; the result is then cut to the limit and
/// `full_len` is exact. Larger ones are only sampled (see
/// [`HUGE_CLOB_PREFIX_BYTES`]).
const WHOLE_READ_FACTOR: u64 = 4;
/// Prefix of a CLOB too large to read at once. The `oracle` crate offers no
/// bounded multi-byte read of more than 100 characters per round trip, so
/// the sample is kept small.
const HUGE_CLOB_PREFIX_BYTES: usize = 2000;
/// Buffer for character reads that is safe for any UTF-8 text: the crate asks
/// the client for as many characters as the buffer has bytes, which only fits
/// when the buffer is small enough to be read through its 400 byte scratch
/// space (a character takes at most 4 bytes).
const SAFE_CHAR_READ_BYTES: usize = 100;

fn oerr(e: oracle::Error) -> Error {
    errors::map_error(&e)
}

fn io_err(e: std::io::Error) -> Error {
    match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<oracle::Error>())
    {
        Some(inner) => errors::map_error(inner),
        None => Error::query(format!("reading a LOB failed: {e}")),
    }
}

// ----------------------------------------------------------------------------
// Column metadata
// ----------------------------------------------------------------------------

pub(crate) fn column_from_info(info: &oracle::ColumnInfo) -> Column {
    let ty = info.oracle_type();
    Column {
        name: info.name().to_owned(),
        type_name: column_type_name(ty),
        kind: column_kind(ty),
        nullable: info.nullable(),
    }
}

/// The type name without length or precision, as in `ALL_TAB_COLUMNS.DATA_TYPE`.
pub(crate) fn column_type_name(t: &OracleType) -> String {
    match t {
        OracleType::Varchar2(_) => "VARCHAR2".into(),
        OracleType::NVarchar2(_) => "NVARCHAR2".into(),
        OracleType::Char(_) => "CHAR".into(),
        OracleType::NChar(_) => "NCHAR".into(),
        OracleType::Rowid => "ROWID".into(),
        OracleType::Raw(_) => "RAW".into(),
        OracleType::BinaryFloat => "BINARY_FLOAT".into(),
        OracleType::BinaryDouble => "BINARY_DOUBLE".into(),
        OracleType::Number(..) | OracleType::Int64 | OracleType::UInt64 => "NUMBER".into(),
        OracleType::Float(_) => "FLOAT".into(),
        OracleType::Date => "DATE".into(),
        OracleType::Timestamp(_) => "TIMESTAMP".into(),
        OracleType::TimestampTZ(_) => "TIMESTAMP WITH TIME ZONE".into(),
        OracleType::TimestampLTZ(_) => "TIMESTAMP WITH LOCAL TIME ZONE".into(),
        OracleType::IntervalDS(..) => "INTERVAL DAY TO SECOND".into(),
        OracleType::IntervalYM(_) => "INTERVAL YEAR TO MONTH".into(),
        OracleType::CLOB => "CLOB".into(),
        OracleType::NCLOB => "NCLOB".into(),
        OracleType::BLOB => "BLOB".into(),
        OracleType::BFILE => "BFILE".into(),
        OracleType::RefCursor => "REF CURSOR".into(),
        OracleType::Boolean => "BOOLEAN".into(),
        OracleType::Object(o) => format!("{}.{}", o.schema(), o.name()),
        OracleType::Long => "LONG".into(),
        OracleType::LongRaw => "LONG RAW".into(),
        OracleType::Json => "JSON".into(),
        OracleType::Xml => "XMLTYPE".into(),
    }
}

pub(crate) fn column_kind(t: &OracleType) -> ValueKind {
    match t {
        OracleType::Varchar2(_)
        | OracleType::NVarchar2(_)
        | OracleType::Char(_)
        | OracleType::NChar(_)
        | OracleType::Long
        | OracleType::Xml
        | OracleType::CLOB
        | OracleType::NCLOB => ValueKind::Text,
        OracleType::Rowid | OracleType::BFILE | OracleType::RefCursor | OracleType::Object(_) => {
            ValueKind::Other
        }
        OracleType::Raw(_) | OracleType::LongRaw | OracleType::BLOB => ValueKind::Bytes,
        OracleType::BinaryFloat | OracleType::BinaryDouble => ValueKind::Float,
        OracleType::Number(prec, scale) => number_kind(*prec, *scale),
        OracleType::Float(_) => ValueKind::Decimal,
        OracleType::Int64 | OracleType::UInt64 => ValueKind::Int,
        // DATE carries a time of day in Oracle.
        OracleType::Date | OracleType::Timestamp(_) => ValueKind::Timestamp,
        // A local time zone value is read in the session time zone with its
        // offset, so it is a value with an offset too.
        OracleType::TimestampTZ(_) | OracleType::TimestampLTZ(_) => ValueKind::TimestampTz,
        OracleType::IntervalDS(..) | OracleType::IntervalYM(_) => ValueKind::Interval,
        OracleType::Boolean => ValueKind::Bool,
        OracleType::Json => ValueKind::Json,
    }
}

// ----------------------------------------------------------------------------
// Numbers
// ----------------------------------------------------------------------------

/// How the values of a NUMBER column are presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberShape {
    /// No fractional digits are declared (integers, `NUMBER`, `FLOAT`):
    /// whole values that fit `i64` are `Int`, everything else `Decimal`.
    Whole,
    /// `NUMBER(p,s)` with `s > 0`: always `Decimal`, padded to `s` digits.
    Fixed(usize),
}

pub(crate) fn number_shape(scale: i8) -> NumberShape {
    if scale > 0 {
        NumberShape::Fixed(scale as usize)
    } else {
        NumberShape::Whole
    }
}

/// `NUMBER(p,0)` that always fits `i64` is an `Int` column; unconstrained
/// `NUMBER` (reported as precision 0) and wider ones are `Decimal` columns.
fn number_kind(prec: u8, scale: i8) -> ValueKind {
    if scale > 0 || prec == 0 || scale == -127 {
        return ValueKind::Decimal;
    }
    if i32::from(prec) - i32::from(scale) <= 18 {
        ValueKind::Int
    } else {
        ValueKind::Decimal
    }
}

/// Plain decimal notation for the text form of an Oracle NUMBER: no exponent,
/// a `0` before a leading point, no trailing fractional zeros, `-` for
/// negatives. `None` for text that is not a finite number (`~` is the Oracle
/// infinity).
pub(crate) fn normalize_number(text: &str) -> Option<String> {
    let t = text.trim();
    let (negative, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (mantissa, exponent) = match t.find(['e', 'E']) {
        Some(i) => (&t[..i], t[i + 1..].parse::<i32>().ok()?),
        None => (t, 0),
    };
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part
        .bytes()
        .chain(frac_part.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    // Oracle's exponent range is far below this; it only bounds the expansion.
    if exponent.abs() > 1000 {
        return None;
    }
    let all_digits = format!("{int_part}{frac_part}");
    let leading_zeros = all_digits.bytes().take_while(|&b| b == b'0').count();
    let digits = all_digits[leading_zeros..].trim_end_matches('0');
    if digits.is_empty() {
        return Some("0".into());
    }
    // Digits to the left of the decimal point.
    let point = int_part.len() as i64 + i64::from(exponent) - leading_zeros as i64;
    let mut out = String::with_capacity(digits.len() + 4);
    if negative {
        out.push('-');
    }
    if point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(digits);
    } else if point as usize >= digits.len() {
        out.push_str(digits);
        out.extend(std::iter::repeat_n('0', point as usize - digits.len()));
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    Some(out)
}

/// Pad the fraction of a normalized number to `scale` digits (`5` -> `5.00`).
pub(crate) fn pad_scale(plain: &str, scale: usize) -> String {
    if scale == 0 {
        return plain.to_owned();
    }
    let (int_part, frac) = plain.split_once('.').unwrap_or((plain, ""));
    let mut out = String::with_capacity(plain.len() + scale + 1);
    out.push_str(int_part);
    out.push('.');
    out.push_str(frac);
    out.extend(std::iter::repeat_n('0', scale.saturating_sub(frac.len())));
    out
}

pub(crate) fn number_value(text: &str, shape: NumberShape) -> Value {
    let Some(plain) = normalize_number(text) else {
        return Value::Other {
            type_name: "NUMBER".into(),
            text: text.trim().to_owned(),
        };
    };
    match shape {
        NumberShape::Fixed(scale) => Value::Decimal(pad_scale(&plain, scale)),
        NumberShape::Whole => {
            if plain.contains('.') {
                Value::Decimal(plain)
            } else {
                plain
                    .parse::<i64>()
                    .map_or_else(|_| Value::Decimal(plain), Value::Int)
            }
        }
    }
}

/// BINARY_FLOAT as the decimal the server prints for it: `0.1f` becomes
/// `0.1`, not `0.10000000149011612`.
pub(crate) fn float32_value(f: f32) -> f64 {
    if f.is_finite() {
        format!("{f}").parse().unwrap_or(f64::from(f))
    } else {
        f64::from(f)
    }
}

// ----------------------------------------------------------------------------
// Dates, timestamps, intervals
// ----------------------------------------------------------------------------

/// DATE and TIMESTAMP (`with_tz == false`), TIMESTAMP WITH [LOCAL] TIME ZONE
/// (`true`). A date the calendar of `chrono` cannot hold comes back as text.
pub(crate) fn timestamp_value(ts: &Timestamp, with_tz: bool) -> Value {
    let naive = NaiveDate::from_ymd_opt(ts.year(), ts.month(), ts.day())
        .and_then(|d| d.and_hms_nano_opt(ts.hour(), ts.minute(), ts.second(), ts.nanosecond()));
    let Some(naive) = naive else {
        return Value::Other {
            type_name: "TIMESTAMP".into(),
            text: ts.to_string(),
        };
    };
    if !with_tz {
        return Value::Timestamp(naive);
    }
    FixedOffset::east_opt(ts.tz_offset())
        .and_then(|offset| offset.from_local_datetime(&naive).single())
        .map_or_else(|| Value::Timestamp(naive), Value::TimestampTz)
}

/// `+01-02` (years-months), the format of `INTERVAL YEAR TO MONTH` literals.
pub(crate) fn interval_ym_text(i: &IntervalYM) -> String {
    i.to_string()
}

/// `+03 04:05:06.000000` (days hours:minutes:seconds.fraction).
pub(crate) fn interval_ds_text(i: &IntervalDS) -> String {
    i.to_string()
}

// ----------------------------------------------------------------------------
// Size limits and LOBs
// ----------------------------------------------------------------------------

/// Text longer than `max` bytes becomes a `Truncated` prefix (cut at a
/// character boundary) with the full byte length.
pub(crate) fn cut_text(mut s: String, max: usize) -> Value {
    if s.len() <= max {
        return Value::Text(s);
    }
    let full = s.len() as u64;
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    Value::Truncated {
        prefix: Box::new(Value::Text(s)),
        full_len: Some(full),
    }
}

pub(crate) fn cut_bytes(mut b: Vec<u8>, max: usize) -> Value {
    if b.len() <= max {
        return Value::Bytes(b);
    }
    let full = b.len() as u64;
    b.truncate(max);
    Value::Truncated {
        prefix: Box::new(Value::Bytes(b)),
        full_len: Some(full),
    }
}

fn utf8_lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// CLOB or NCLOB as text, at most `max_bytes` of it.
///
/// LOBs up to `WHOLE_READ_FACTOR * max_bytes` characters are read in one
/// call, so the prefix has the full limit and `full_len` is the exact byte
/// length. For larger ones `full_len` is the LOB length in UTF-16 units (a
/// lower bound of the byte length) and the prefix is a short sample.
pub(crate) fn read_text_lob<L: Read + Lob>(lob: &mut L, max_bytes: usize) -> Result<Value> {
    let size = lob.size().map_err(oerr)?;
    if size == 0 {
        return Ok(Value::Text(String::new()));
    }
    if size <= (max_bytes as u64).saturating_mul(WHOLE_READ_FACTOR) {
        let mut bytes = Vec::new();
        lob.read_to_end(&mut bytes).map_err(io_err)?;
        return Ok(cut_text(utf8_lossy(bytes), max_bytes));
    }
    let want = HUGE_CLOB_PREFIX_BYTES.min(max_bytes);
    let mut sample = Vec::with_capacity(want + SAFE_CHAR_READ_BYTES);
    let mut buf = [0u8; SAFE_CHAR_READ_BYTES];
    while sample.len() < want {
        let n = lob.read(&mut buf).map_err(io_err)?;
        if n == 0 {
            break;
        }
        sample.extend_from_slice(&buf[..n]);
    }
    let prefix = match cut_text(utf8_lossy(sample), max_bytes.min(want)) {
        Value::Truncated { prefix, .. } => *prefix,
        other => other,
    };
    Ok(Value::Truncated {
        prefix: Box::new(prefix),
        full_len: Some(size),
    })
}

/// BLOB as bytes, at most `max_bytes` of it; `full_len` is the exact size.
pub(crate) fn read_bytes_lob<L: Read + Lob>(lob: &mut L, max_bytes: usize) -> Result<Value> {
    let size = lob.size().map_err(oerr)?;
    if size == 0 {
        return Ok(Value::Bytes(Vec::new()));
    }
    let take = size.min(max_bytes as u64) as usize;
    let mut buf = vec![0u8; take];
    let mut filled = 0;
    while filled < take {
        let n = lob.read(&mut buf[filled..]).map_err(io_err)?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(if size > max_bytes as u64 {
        Value::Truncated {
            prefix: Box::new(Value::Bytes(buf)),
            full_len: Some(size),
        }
    } else {
        Value::Bytes(buf)
    })
}

// ----------------------------------------------------------------------------
// Fetched rows
// ----------------------------------------------------------------------------

fn text_of(inner: &InnerValue<'_>) -> Option<String> {
    match inner {
        InnerValue::Char(b) => Some(String::from_utf8_lossy(b).into_owned()),
        InnerValue::Number(s) => Some((*s).to_owned()),
        _ => None,
    }
}

fn number_cell(sv: &oracle::SqlValue<'_>, shape: NumberShape) -> Result<Value> {
    Ok(match sv.as_inner_value().map_err(oerr)? {
        InnerValue::Int64(i) => match shape {
            NumberShape::Whole => Value::Int(i),
            NumberShape::Fixed(scale) => Value::Decimal(pad_scale(&i.to_string(), scale)),
        },
        InnerValue::UInt64(u) => number_value(&u.to_string(), shape),
        InnerValue::Float(f) => Value::Float(float32_value(f)),
        InnerValue::Double(d) => Value::Float(d),
        other => match text_of(&other) {
            Some(text) => number_value(&text, shape),
            None => Value::Text(sv.get::<String>().map_err(oerr)?),
        },
    })
}

/// One fetched value as a `Value`, honouring `max` bytes for text, binary
/// and LOB values.
pub(crate) fn convert_cell(
    sv: &oracle::SqlValue<'_>,
    ty: &OracleType,
    max: usize,
) -> Result<Value> {
    if sv.is_null().map_err(oerr)? {
        return Ok(Value::Null);
    }
    match ty {
        OracleType::Varchar2(_)
        | OracleType::NVarchar2(_)
        | OracleType::Char(_)
        | OracleType::NChar(_)
        | OracleType::Long
        | OracleType::Xml
        | OracleType::Json => {
            let text = match sv.as_inner_value().map_err(oerr)? {
                InnerValue::Char(b) => String::from_utf8_lossy(b).into_owned(),
                _ => sv.get::<String>().map_err(oerr)?,
            };
            Ok(match ty {
                OracleType::Json => json_value(text, max),
                _ => cut_text(text, max),
            })
        }
        OracleType::Number(_, scale) => number_cell(sv, number_shape(*scale)),
        OracleType::Float(_) | OracleType::Int64 | OracleType::UInt64 => {
            number_cell(sv, NumberShape::Whole)
        }
        OracleType::BinaryFloat | OracleType::BinaryDouble => number_cell(sv, NumberShape::Whole),
        OracleType::Raw(_) | OracleType::LongRaw => match sv.as_inner_value().map_err(oerr)? {
            InnerValue::Raw(b) => Ok(cut_bytes(b.to_vec(), max)),
            _ => Ok(cut_bytes(sv.get::<Vec<u8>>().map_err(oerr)?, max)),
        },
        OracleType::Date | OracleType::Timestamp(_) => {
            let ts: Timestamp = sv.get().map_err(oerr)?;
            Ok(timestamp_value(&ts, false))
        }
        OracleType::TimestampTZ(_) | OracleType::TimestampLTZ(_) => {
            let ts: Timestamp = sv.get().map_err(oerr)?;
            Ok(timestamp_value(&ts, true))
        }
        OracleType::IntervalDS(..) => {
            let i: IntervalDS = sv.get().map_err(oerr)?;
            Ok(Value::Interval(interval_ds_text(&i)))
        }
        OracleType::IntervalYM(_) => {
            let i: IntervalYM = sv.get().map_err(oerr)?;
            Ok(Value::Interval(interval_ym_text(&i)))
        }
        OracleType::CLOB => {
            let mut lob: Clob = sv.get().map_err(oerr)?;
            read_text_lob(&mut lob, max)
        }
        OracleType::NCLOB => {
            let mut lob: Nclob = sv.get().map_err(oerr)?;
            read_text_lob(&mut lob, max)
        }
        OracleType::BLOB => {
            let mut lob: Blob = sv.get().map_err(oerr)?;
            read_bytes_lob(&mut lob, max)
        }
        // The file behind a BFILE is not read: it lives on the database server.
        OracleType::BFILE => Ok(Value::Other {
            type_name: "BFILE".into(),
            text: "(BFILE)".into(),
        }),
        OracleType::Rowid => Ok(Value::Other {
            type_name: "ROWID".into(),
            text: sv.get::<String>().map_err(oerr)?,
        }),
        OracleType::Boolean => match sv.as_inner_value().map_err(oerr)? {
            InnerValue::Boolean(b) => Ok(Value::Bool(b)),
            _ => Ok(Value::Bool(sv.get::<bool>().map_err(oerr)?)),
        },
        OracleType::RefCursor => Ok(Value::Other {
            type_name: "REF CURSOR".into(),
            text: "(cursor)".into(),
        }),
        OracleType::Object(_) => Ok(Value::Other {
            type_name: column_type_name(ty),
            text: sv.get::<String>().map_err(oerr)?,
        }),
    }
}

fn json_value(text: String, max: usize) -> Value {
    match cut_text(text, max) {
        Value::Text(t) => Value::Json(t),
        Value::Truncated { prefix, full_len } => match *prefix {
            Value::Text(t) => Value::Truncated {
                prefix: Box::new(Value::Json(t)),
                full_len,
            },
            other => Value::Truncated {
                prefix: Box::new(other),
                full_len,
            },
        },
        other => other,
    }
}

thread_local! {
    static MAX_VALUE_BYTES: Cell<usize> = const { Cell::new(1 << 20) };
}

/// The size limit for the next fetch on this thread. Row conversion runs
/// inside the `oracle` crate's iterator, which passes no context, so the
/// worker thread sets the limit before it fetches.
pub(crate) fn set_max_value_bytes(n: usize) {
    MAX_VALUE_BYTES.with(|c| c.set(n));
}

/// A fetched row converted to values. `get` cannot fail with our own error
/// type, so a conversion error travels in `error`.
pub(crate) struct ConvertedRow {
    pub values: Vec<Value>,
    pub error: Option<Error>,
}

impl oracle::RowValue for ConvertedRow {
    fn get(row: &oracle::Row) -> oracle::Result<Self> {
        let max = MAX_VALUE_BYTES.with(Cell::get);
        let infos = row.column_info();
        let mut values = Vec::with_capacity(infos.len());
        for (info, sv) in infos.iter().zip(row.sql_values()) {
            match convert_cell(sv, info.oracle_type(), max) {
                Ok(v) => values.push(v),
                Err(e) => {
                    return Ok(ConvertedRow {
                        values,
                        error: Some(e),
                    });
                }
            }
        }
        Ok(ConvertedRow {
            values,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use chrono::{Datelike, Timelike};

    use super::*;

    #[test]
    fn type_names_have_no_sizes() {
        assert_eq!(column_type_name(&OracleType::Varchar2(50)), "VARCHAR2");
        assert_eq!(column_type_name(&OracleType::Number(10, 2)), "NUMBER");
        assert_eq!(column_type_name(&OracleType::Timestamp(6)), "TIMESTAMP");
        assert_eq!(
            column_type_name(&OracleType::TimestampTZ(3)),
            "TIMESTAMP WITH TIME ZONE"
        );
        assert_eq!(
            column_type_name(&OracleType::TimestampLTZ(9)),
            "TIMESTAMP WITH LOCAL TIME ZONE"
        );
        assert_eq!(
            column_type_name(&OracleType::IntervalDS(2, 6)),
            "INTERVAL DAY TO SECOND"
        );
        assert_eq!(column_type_name(&OracleType::LongRaw), "LONG RAW");
        assert_eq!(column_type_name(&OracleType::Xml), "XMLTYPE");
    }

    #[test]
    fn column_kinds() {
        use ValueKind as K;
        let k = column_kind;
        assert_eq!(k(&OracleType::Varchar2(10)), K::Text);
        assert_eq!(k(&OracleType::NChar(10)), K::Text);
        assert_eq!(k(&OracleType::Long), K::Text);
        assert_eq!(k(&OracleType::CLOB), K::Text);
        assert_eq!(k(&OracleType::NCLOB), K::Text);
        assert_eq!(k(&OracleType::Raw(16)), K::Bytes);
        assert_eq!(k(&OracleType::LongRaw), K::Bytes);
        assert_eq!(k(&OracleType::BLOB), K::Bytes);
        assert_eq!(k(&OracleType::BinaryFloat), K::Float);
        assert_eq!(k(&OracleType::BinaryDouble), K::Float);
        assert_eq!(k(&OracleType::Date), K::Timestamp);
        assert_eq!(k(&OracleType::Timestamp(6)), K::Timestamp);
        assert_eq!(k(&OracleType::TimestampTZ(6)), K::TimestampTz);
        assert_eq!(k(&OracleType::TimestampLTZ(6)), K::TimestampTz);
        assert_eq!(k(&OracleType::IntervalYM(2)), K::Interval);
        assert_eq!(k(&OracleType::IntervalDS(2, 6)), K::Interval);
        assert_eq!(k(&OracleType::Rowid), K::Other);
        assert_eq!(k(&OracleType::BFILE), K::Other);
        assert_eq!(k(&OracleType::RefCursor), K::Other);
        assert_eq!(k(&OracleType::Boolean), K::Bool);
        assert_eq!(k(&OracleType::Json), K::Json);
    }

    #[test]
    fn number_column_kinds() {
        use ValueKind as K;
        let k = |p, s| column_kind(&OracleType::Number(p, s));
        assert_eq!(k(10, 0), K::Int);
        assert_eq!(k(18, 0), K::Int);
        assert_eq!(k(19, 0), K::Decimal);
        assert_eq!(k(38, 0), K::Decimal);
        assert_eq!(k(10, 2), K::Decimal);
        assert_eq!(k(5, -2), K::Int);
        assert_eq!(k(0, -127), K::Decimal); // NUMBER
        assert_eq!(k(0, 0), K::Decimal); // NUMBER(*,0)
        assert_eq!(column_kind(&OracleType::Float(126)), K::Decimal);
    }

    #[test]
    fn normalizes_plain_numbers() {
        let n = |s: &str| normalize_number(s);
        assert_eq!(n("0").as_deref(), Some("0"));
        assert_eq!(n("1").as_deref(), Some("1"));
        assert_eq!(n("-1").as_deref(), Some("-1"));
        assert_eq!(n("12.5").as_deref(), Some("12.5"));
        assert_eq!(n("-12.50").as_deref(), Some("-12.5"));
        assert_eq!(n(".5").as_deref(), Some("0.5"));
        assert_eq!(n("-.5").as_deref(), Some("-0.5"));
        assert_eq!(n("0.05").as_deref(), Some("0.05"));
        assert_eq!(n("000123").as_deref(), Some("123"));
        assert_eq!(n("+7").as_deref(), Some("7"));
        assert_eq!(n("  42 ").as_deref(), Some("42"));
        assert_eq!(n("100").as_deref(), Some("100"));
        assert_eq!(n("0.000").as_deref(), Some("0"));
        assert_eq!(n("-0").as_deref(), Some("0"));
    }

    #[test]
    fn removes_exponents() {
        let n = |s: &str| normalize_number(s);
        assert_eq!(n("1E+2").as_deref(), Some("100"));
        assert_eq!(n("1e2").as_deref(), Some("100"));
        assert_eq!(n("1.5E+3").as_deref(), Some("1500"));
        assert_eq!(n("1.5E-5").as_deref(), Some("0.000015"));
        assert_eq!(n("-2.5E-1").as_deref(), Some("-0.25"));
        assert_eq!(n("1.2345E+2").as_deref(), Some("123.45"));
        assert_eq!(n("0E+5").as_deref(), Some("0"));
        assert_eq!(
            n("1E+38").as_deref(),
            Some("100000000000000000000000000000000000000")
        );
        assert_eq!(n("-1.5E-10").as_deref(), Some("-0.00000000015"));
        // 38 significant digits are kept exactly.
        assert_eq!(
            n("12345678901234567890123456789012345678").as_deref(),
            Some("12345678901234567890123456789012345678")
        );
        assert_eq!(
            n("1.2345678901234567890123456789012345678E+10").as_deref(),
            Some("12345678901.234567890123456789012345678")
        );
    }

    #[test]
    fn rejects_non_numbers() {
        for s in [
            "", "-", "~", "-~", "NaN", "1.2.3", "1e", "1e+x", "abc", "1,5", ".", "1E+99999",
        ] {
            assert_eq!(normalize_number(s), None, "{s:?}");
        }
    }

    #[test]
    fn whole_numbers_are_int_when_they_fit() {
        let v = |s| number_value(s, NumberShape::Whole);
        assert_eq!(v("42"), Value::Int(42));
        assert_eq!(v("-7"), Value::Int(-7));
        assert_eq!(v("9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(v("-9223372036854775808"), Value::Int(i64::MIN));
        assert_eq!(
            v("9223372036854775808"),
            Value::Decimal("9223372036854775808".into())
        );
        assert_eq!(
            v("12345678901234567890123456789012345678"),
            Value::Decimal("12345678901234567890123456789012345678".into())
        );
        assert_eq!(v("1E+3"), Value::Int(1000));
        // Unconstrained NUMBER holding a fraction.
        assert_eq!(v("0.5"), Value::Decimal("0.5".into()));
        assert_eq!(v(".5"), Value::Decimal("0.5".into()));
        assert_eq!(v("-.25"), Value::Decimal("-0.25".into()));
        // Not a finite number: kept as text with its type.
        assert_eq!(
            v("~"),
            Value::Other {
                type_name: "NUMBER".into(),
                text: "~".into()
            }
        );
    }

    #[test]
    fn fixed_scale_numbers_are_padded_decimals() {
        let v = |s, scale| number_value(s, NumberShape::Fixed(scale));
        assert_eq!(v("12.5", 2), Value::Decimal("12.50".into()));
        assert_eq!(v("5", 2), Value::Decimal("5.00".into()));
        assert_eq!(v("0", 2), Value::Decimal("0.00".into()));
        assert_eq!(v(".5", 3), Value::Decimal("0.500".into()));
        assert_eq!(v("-0.05", 2), Value::Decimal("-0.05".into()));
        assert_eq!(
            v("1.234", 2),
            Value::Decimal("1.234".into()),
            "never truncates"
        );
        assert_eq!(number_shape(2), NumberShape::Fixed(2));
        assert_eq!(number_shape(0), NumberShape::Whole);
        assert_eq!(number_shape(-2), NumberShape::Whole);
        assert_eq!(number_shape(-127), NumberShape::Whole);
    }

    #[test]
    fn binary_float_prints_like_the_server() {
        assert_eq!(float32_value(0.1), 0.1);
        assert_eq!(float32_value(1.5), 1.5);
        assert_eq!(float32_value(-3.25), -3.25);
        assert!(float32_value(f32::NAN).is_nan());
        assert_eq!(float32_value(f32::INFINITY), f64::INFINITY);
    }

    #[test]
    fn timestamps() {
        let ts = Timestamp::new(2026, 3, 4, 5, 6, 7, 123_456_000).unwrap();
        match timestamp_value(&ts, false) {
            Value::Timestamp(t) => {
                assert_eq!((t.year(), t.month(), t.day()), (2026, 3, 4));
                assert_eq!((t.hour(), t.minute(), t.second()), (5, 6, 7));
                assert_eq!(t.nanosecond(), 123_456_000);
            }
            other => panic!("{other:?}"),
        }
        let tz = ts.and_tz_hm_offset(-5, -30).unwrap();
        match timestamp_value(&tz, true) {
            Value::TimestampTz(t) => {
                assert_eq!(t.offset().local_minus_utc(), -(5 * 3600 + 30 * 60));
                assert_eq!(t.hour(), 5);
                assert_eq!(t.to_rfc3339(), "2026-03-04T05:06:07.123456-05:30");
            }
            other => panic!("{other:?}"),
        }
        let utc = ts.and_tz_hm_offset(0, 0).unwrap();
        match timestamp_value(&utc, true) {
            Value::TimestampTz(t) => assert_eq!(t.offset().local_minus_utc(), 0),
            other => panic!("{other:?}"),
        }
        // Midnight DATE.
        let d = Timestamp::new(1999, 12, 31, 0, 0, 0, 0).unwrap();
        assert!(matches!(timestamp_value(&d, false), Value::Timestamp(_)));
    }

    #[test]
    fn intervals_use_the_server_text_form() {
        let ds = IntervalDS::new(3, 4, 5, 6, 0)
            .unwrap()
            .and_prec(2, 6)
            .unwrap();
        assert_eq!(interval_ds_text(&ds), "+03 04:05:06.000000");
        let ds = IntervalDS::new(-1, -2, -3, -4, -500_000_000)
            .unwrap()
            .and_prec(2, 6)
            .unwrap();
        assert_eq!(interval_ds_text(&ds), "-01 02:03:04.500000");
        let ym = IntervalYM::new(1, 2).unwrap().and_prec(2).unwrap();
        assert_eq!(interval_ym_text(&ym), "+01-02");
        let ym = IntervalYM::new(-10, -11).unwrap().and_prec(2).unwrap();
        assert_eq!(interval_ym_text(&ym), "-10-11");
    }

    #[test]
    fn text_is_cut_at_a_character_boundary() {
        assert_eq!(cut_text("abc".into(), 3), Value::Text("abc".into()));
        let cut = cut_text("abcdef".into(), 4);
        assert_eq!(
            cut,
            Value::Truncated {
                prefix: Box::new(Value::Text("abcd".into())),
                full_len: Some(6)
            }
        );
        // 'é' is two bytes; 3 bytes of "ééé" keep one character.
        match cut_text("ééé".into(), 3) {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(*prefix, Value::Text("é".into()));
                assert_eq!(full_len, Some(6));
            }
            other => panic!("{other:?}"),
        }
        // A limit inside the first character leaves an empty prefix.
        match cut_text("ééé".into(), 1) {
            Value::Truncated { prefix, .. } => assert_eq!(*prefix, Value::Text(String::new())),
            other => panic!("{other:?}"),
        }
        assert_eq!(cut_bytes(vec![1, 2, 3], 3), Value::Bytes(vec![1, 2, 3]));
        assert_eq!(
            cut_bytes(vec![1, 2, 3, 4], 2),
            Value::Truncated {
                prefix: Box::new(Value::Bytes(vec![1, 2])),
                full_len: Some(4)
            }
        );
    }

    #[test]
    fn json_keeps_its_kind_when_cut() {
        assert_eq!(json_value("{}".into(), 10), Value::Json("{}".into()));
        match json_value("{\"a\":1}".into(), 3) {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(*prefix, Value::Json("{\"a".into()));
                assert_eq!(full_len, Some(7));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A LOB in memory that behaves like the `oracle` crate's `Clob` for the
    /// cases the readers depend on: sizes in UTF-16 units, reads that return
    /// whole characters, `read_to_end` as one call.
    struct MemLob {
        data: Vec<u8>,
        pos: usize,
        units: u64,
        text: bool,
        reads: usize,
    }

    impl MemLob {
        fn text(s: &str) -> Self {
            MemLob {
                data: s.as_bytes().to_vec(),
                pos: 0,
                units: s.encode_utf16().count() as u64,
                text: true,
                reads: 0,
            }
        }

        fn binary(b: Vec<u8>) -> Self {
            MemLob {
                units: b.len() as u64,
                data: b,
                pos: 0,
                text: false,
                reads: 0,
            }
        }
    }

    impl Read for MemLob {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            let mut n = buf.len().min(self.data.len() - self.pos);
            if self.text {
                // Whole characters only.
                while n > 0
                    && (self.data[self.pos + n..]
                        .first()
                        .is_some_and(|b| b & 0xC0 == 0x80))
                {
                    n -= 1;
                }
            }
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl Lob for MemLob {
        fn size(&self) -> oracle::Result<u64> {
            Ok(self.units)
        }
        fn truncate(&mut self, _new_size: u64) -> oracle::Result<()> {
            Ok(())
        }
        fn chunk_size(&self) -> oracle::Result<usize> {
            Ok(8192)
        }
        fn open_resource(&mut self) -> oracle::Result<()> {
            Ok(())
        }
        fn close_resource(&mut self) -> oracle::Result<()> {
            Ok(())
        }
        fn is_resource_open(&self) -> oracle::Result<bool> {
            Ok(false)
        }
    }

    #[test]
    fn small_clob_is_read_whole() {
        let mut lob = MemLob::text("hello ёжик");
        let v = read_text_lob(&mut lob, 1 << 20).unwrap();
        assert_eq!(v, Value::Text("hello ёжик".into()));
        let mut empty = MemLob::text("");
        assert_eq!(
            read_text_lob(&mut empty, 10).unwrap(),
            Value::Text(String::new())
        );
        assert_eq!(empty.reads, 0, "an empty LOB needs no read");
    }

    #[test]
    fn two_mib_clob_is_cut_to_the_limit_with_its_exact_length() {
        let text = "x".repeat(2 * 1024 * 1024);
        let mut lob = MemLob::text(&text);
        match read_text_lob(&mut lob, 1 << 20).unwrap() {
            Value::Truncated { prefix, full_len } => {
                match *prefix {
                    Value::Text(t) => assert_eq!(t.len(), 1 << 20),
                    other => panic!("{other:?}"),
                }
                assert_eq!(full_len, Some(2 * 1024 * 1024));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn multibyte_clob_reports_the_byte_length_when_read_whole() {
        // 400 Cyrillic characters are 800 bytes; the limit is 500.
        let text = "ж".repeat(400);
        let mut lob = MemLob::text(&text);
        match read_text_lob(&mut lob, 500).unwrap() {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(full_len, Some(800));
                match *prefix {
                    Value::Text(t) => {
                        assert_eq!(t.len(), 500);
                        assert!(t.chars().all(|c| c == 'ж'));
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn huge_clob_is_sampled_in_safe_reads() {
        // Larger than WHOLE_READ_FACTOR * max: only a short prefix is read,
        // never more than 100 bytes per call, and never a split character.
        let text = "я".repeat(5000);
        let mut lob = MemLob::text(&text);
        match read_text_lob(&mut lob, 1000).unwrap() {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(full_len, Some(5000));
                match *prefix {
                    Value::Text(t) => {
                        assert!(t.len() >= 1000 - 2 && t.len() <= 1000, "{}", t.len());
                        assert!(t.chars().all(|c| c == 'я'));
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        assert!(lob.reads <= 12, "{} reads", lob.reads);
    }

    #[test]
    fn blob_prefix_and_full_length() {
        let data: Vec<u8> = (0..=255u8).cycle().take(2 * 1024 * 1024).collect();
        let mut lob = MemLob::binary(data.clone());
        match read_bytes_lob(&mut lob, 1 << 20).unwrap() {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(full_len, Some(2 * 1024 * 1024));
                assert_eq!(*prefix, Value::Bytes(data[..1 << 20].to_vec()));
            }
            other => panic!("{other:?}"),
        }
        let mut small = MemLob::binary(vec![1, 2, 3]);
        assert_eq!(
            read_bytes_lob(&mut small, 10).unwrap(),
            Value::Bytes(vec![1, 2, 3])
        );
        let mut empty = MemLob::binary(Vec::new());
        assert_eq!(
            read_bytes_lob(&mut empty, 10).unwrap(),
            Value::Bytes(Vec::new())
        );
        let mut exact = MemLob::binary(vec![9; 10]);
        assert_eq!(
            read_bytes_lob(&mut exact, 10).unwrap(),
            Value::Bytes(vec![9; 10])
        );
    }
}
