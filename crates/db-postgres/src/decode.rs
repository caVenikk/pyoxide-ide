//! Binary result decoding: PostgreSQL wire values to [`Value`], by type OID.
//!
//! Written from the public PostgreSQL documentation (data type chapters and the
//! protocol description). tokio-postgres asks for binary results for every
//! column, so the formats here are the `*_send` formats of each type.

use std::net::{Ipv4Addr, Ipv6Addr};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use db_core::{Column, Value, ValueKind};
use tokio_postgres::types::{FromSql, Kind, Type};

use crate::{interval, numeric};

/// Well-known type OIDs from `pg_type`.
mod oid {
    pub const BOOL: u32 = 16;
    pub const BYTEA: u32 = 17;
    pub const CHAR: u32 = 18;
    pub const NAME: u32 = 19;
    pub const INT8: u32 = 20;
    pub const INT2: u32 = 21;
    pub const INT2VECTOR: u32 = 22;
    pub const INT4: u32 = 23;
    pub const REGPROC: u32 = 24;
    pub const TEXT: u32 = 25;
    pub const OID: u32 = 26;
    pub const TID: u32 = 27;
    pub const XID: u32 = 28;
    pub const CID: u32 = 29;
    pub const OIDVECTOR: u32 = 30;
    pub const JSON: u32 = 114;
    pub const XML: u32 = 142;
    pub const CIDR: u32 = 650;
    pub const FLOAT4: u32 = 700;
    pub const FLOAT8: u32 = 701;
    pub const UNKNOWN: u32 = 705;
    pub const MACADDR8: u32 = 774;
    pub const MONEY: u32 = 790;
    pub const MACADDR: u32 = 829;
    pub const INET: u32 = 869;
    pub const BPCHAR: u32 = 1042;
    pub const VARCHAR: u32 = 1043;
    pub const DATE: u32 = 1082;
    pub const TIME: u32 = 1083;
    pub const TIMESTAMP: u32 = 1114;
    pub const TIMESTAMPTZ: u32 = 1184;
    pub const INTERVAL: u32 = 1186;
    pub const TIMETZ: u32 = 1266;
    pub const BIT: u32 = 1560;
    pub const VARBIT: u32 = 1562;
    pub const NUMERIC: u32 = 1700;
    pub const REGPROCEDURE: u32 = 2202;
    pub const REGOPER: u32 = 2203;
    pub const REGOPERATOR: u32 = 2204;
    pub const REGCLASS: u32 = 2205;
    pub const REGTYPE: u32 = 2206;
    pub const RECORD: u32 = 2249;
    pub const UUID: u32 = 2950;
    pub const PG_LSN: u32 = 3220;
    pub const REGCONFIG: u32 = 3734;
    pub const REGDICTIONARY: u32 = 3769;
    pub const JSONB: u32 = 3802;
    pub const JSONPATH: u32 = 4072;
    pub const REGNAMESPACE: u32 = 4089;
    pub const REGROLE: u32 = 4096;
    pub const REGCOLLATION: u32 = 4191;
    pub const XID8: u32 = 5069;
}

/// Days from 0001-01-01 are not needed; PostgreSQL counts from 2000-01-01.
fn pg_epoch() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .expect("2000-01-01 is a valid date")
}

/// The `Column` description for a prepared statement's result column.
pub(crate) fn column_info(col: &tokio_postgres::Column) -> Column {
    let ty = col.type_();
    Column {
        name: col.name().to_owned(),
        type_name: type_name(ty),
        kind: kind_of(ty),
        nullable: true,
    }
}

pub(crate) fn type_name(ty: &Type) -> String {
    match ty.kind() {
        Kind::Array(elem) => format!("{}[]", type_name(elem)),
        _ => ty.name().to_owned(),
    }
}

/// The shape of the values of a type, for alignment and editors.
pub(crate) fn kind_of(ty: &Type) -> ValueKind {
    match ty.kind() {
        Kind::Array(_) => return ValueKind::Array,
        Kind::Domain(base) => return kind_of(base),
        Kind::Enum(_) => return ValueKind::Text,
        Kind::Range(_) | Kind::Multirange(_) | Kind::Composite(_) | Kind::Pseudo => {
            return ValueKind::Other;
        }
        Kind::Simple => {}
        _ => return ValueKind::Other,
    }
    match ty.oid() {
        oid::BOOL => ValueKind::Bool,
        oid::INT2
        | oid::INT4
        | oid::INT8
        | oid::OID
        | oid::XID
        | oid::CID
        | oid::REGPROC
        | oid::REGPROCEDURE
        | oid::REGOPER
        | oid::REGOPERATOR
        | oid::REGCLASS
        | oid::REGTYPE
        | oid::REGCONFIG
        | oid::REGDICTIONARY
        | oid::REGNAMESPACE
        | oid::REGROLE
        | oid::REGCOLLATION => ValueKind::Int,
        oid::FLOAT4 | oid::FLOAT8 => ValueKind::Float,
        oid::NUMERIC | oid::XID8 => ValueKind::Decimal,
        oid::TEXT
        | oid::VARCHAR
        | oid::BPCHAR
        | oid::NAME
        | oid::CHAR
        | oid::UNKNOWN
        | oid::XML => ValueKind::Text,
        oid::BYTEA => ValueKind::Bytes,
        oid::DATE => ValueKind::Date,
        oid::TIME => ValueKind::Time,
        oid::TIMESTAMP => ValueKind::Timestamp,
        oid::TIMESTAMPTZ => ValueKind::TimestampTz,
        oid::INTERVAL => ValueKind::Interval,
        oid::UUID => ValueKind::Uuid,
        oid::JSON | oid::JSONB => ValueKind::Json,
        oid::INT2VECTOR | oid::OIDVECTOR => ValueKind::Array,
        _ if ty.name() == "citext" => ValueKind::Text,
        _ => ValueKind::Other,
    }
}

/// A cell as it came off the wire: `None` is SQL NULL.
pub(crate) struct RawCell<'a>(pub Option<&'a [u8]>);

impl<'a> FromSql<'a> for RawCell<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(RawCell(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(RawCell(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// Convert one result row. Values longer than `limit` bytes are truncated.
pub(crate) fn convert_row(
    row: &tokio_postgres::Row,
    limit: usize,
) -> Result<Vec<Value>, tokio_postgres::Error> {
    let columns = row.columns();
    let mut out = Vec::with_capacity(columns.len());
    for (i, col) in columns.iter().enumerate() {
        let cell: RawCell<'_> = row.try_get(i)?;
        out.push(decode_cell(col.type_(), cell.0, limit));
    }
    Ok(out)
}

/// Decode one cell. Never fails: undecodable data falls back to
/// [`Value::Other`] with text or hex.
pub(crate) fn decode_cell(ty: &Type, raw: Option<&[u8]>, limit: usize) -> Value {
    let Some(raw) = raw else {
        return Value::Null;
    };
    decode(ty, raw, limit).unwrap_or_else(|| fallback(ty, raw))
}

fn decode(ty: &Type, raw: &[u8], limit: usize) -> Option<Value> {
    match ty.kind() {
        Kind::Array(elem) => return decode_array(elem, raw, limit),
        Kind::Domain(base) => return decode(base, raw, limit),
        Kind::Enum(_) => return Some(text_value(raw, limit)),
        Kind::Range(sub) => return decode_range(sub, raw).map(|t| other(ty, t)),
        Kind::Multirange(sub) => return decode_multirange(sub, raw).map(|t| other(ty, t)),
        Kind::Composite(fields) => {
            let types: Vec<Type> = fields.iter().map(|f| f.type_().clone()).collect();
            return decode_composite(Some(&types), raw).map(|t| other(ty, t));
        }
        Kind::Simple | Kind::Pseudo => {}
        _ => return None,
    }

    Some(match ty.oid() {
        oid::BOOL => Value::Bool(*raw.first()? != 0),
        oid::INT2 => Value::Int(i64::from(i16::from_be_bytes(raw.try_into().ok()?))),
        oid::INT4 => Value::Int(i64::from(i32::from_be_bytes(raw.try_into().ok()?))),
        oid::INT8 => Value::Int(i64::from_be_bytes(raw.try_into().ok()?)),
        oid::OID
        | oid::XID
        | oid::CID
        | oid::REGPROC
        | oid::REGPROCEDURE
        | oid::REGOPER
        | oid::REGOPERATOR
        | oid::REGCLASS
        | oid::REGTYPE
        | oid::REGCONFIG
        | oid::REGDICTIONARY
        | oid::REGNAMESPACE
        | oid::REGROLE
        | oid::REGCOLLATION => Value::Int(i64::from(u32::from_be_bytes(raw.try_into().ok()?))),
        oid::XID8 => Value::Decimal(u64::from_be_bytes(raw.try_into().ok()?).to_string()),
        oid::FLOAT4 => Value::Float(shortest_f32(f32::from_be_bytes(raw.try_into().ok()?))),
        oid::FLOAT8 => Value::Float(f64::from_be_bytes(raw.try_into().ok()?)),
        oid::NUMERIC => match numeric::decode(raw)? {
            numeric::Numeric::Number(s) => Value::Decimal(s),
            special => Value::Other {
                type_name: "numeric".to_owned(),
                text: special.special_text()?.to_owned(),
            },
        },
        oid::TEXT | oid::VARCHAR | oid::BPCHAR | oid::NAME | oid::UNKNOWN | oid::XML => {
            text_value(raw, limit)
        }
        oid::CHAR => Value::Text(String::from_utf8_lossy(raw).into_owned()),
        oid::BYTEA => bytes_value(raw, limit),
        oid::UUID => Value::Uuid(uuid_text(raw)?),
        oid::JSON => json_value(raw, limit),
        oid::JSONB => {
            let (version, body) = raw.split_first()?;
            if *version != 1 {
                return None;
            }
            json_value(body, limit)
        }
        oid::DATE => {
            let days = i32::from_be_bytes(raw.try_into().ok()?);
            match days {
                i32::MAX => other_named("date", "infinity"),
                i32::MIN => other_named("date", "-infinity"),
                _ => Value::Date(
                    pg_epoch()
                        .date()
                        .checked_add_signed(TimeDelta::try_days(i64::from(days))?)?,
                ),
            }
        }
        oid::TIME => {
            let micros = i64::from_be_bytes(raw.try_into().ok()?);
            match time_of_day(micros) {
                Some(t) => Value::Time(t),
                None => other_named("time", &format_time_micros(micros)),
            }
        }
        oid::TIMETZ => {
            if raw.len() != 12 {
                return None;
            }
            let micros = i64::from_be_bytes(raw[0..8].try_into().ok()?);
            let west = i32::from_be_bytes(raw[8..12].try_into().ok()?);
            other_named("timetz", &format_timetz(micros, west))
        }
        oid::TIMESTAMP => {
            let micros = i64::from_be_bytes(raw.try_into().ok()?);
            match micros {
                i64::MAX => other_named("timestamp", "infinity"),
                i64::MIN => other_named("timestamp", "-infinity"),
                _ => Value::Timestamp(timestamp(micros)?),
            }
        }
        oid::TIMESTAMPTZ => {
            let micros = i64::from_be_bytes(raw.try_into().ok()?);
            match micros {
                i64::MAX => other_named("timestamptz", "infinity"),
                i64::MIN => other_named("timestamptz", "-infinity"),
                _ => Value::TimestampTz(DateTime::from_naive_utc_and_offset(
                    timestamp(micros)?,
                    FixedOffset::east_opt(0)?,
                )),
            }
        }
        oid::INTERVAL => Value::Interval(interval::decode(raw)?),
        oid::INT2VECTOR => return decode_array(&Type::INT2, raw, limit),
        oid::OIDVECTOR => return decode_array(&Type::OID, raw, limit),
        oid::INET | oid::CIDR => other(ty, inet_text(raw)?),
        oid::MACADDR => other(ty, mac_text(raw, 6)?),
        oid::MACADDR8 => other(ty, mac_text(raw, 8)?),
        oid::BIT | oid::VARBIT => other(ty, bit_text(raw)?),
        oid::MONEY => other(ty, money_text(i64::from_be_bytes(raw.try_into().ok()?))),
        oid::PG_LSN => {
            let v = u64::from_be_bytes(raw.try_into().ok()?);
            other(ty, format!("{:X}/{:X}", v >> 32, v & 0xFFFF_FFFF))
        }
        oid::TID => {
            if raw.len() != 6 {
                return None;
            }
            let block = u32::from_be_bytes(raw[0..4].try_into().ok()?);
            let offset = u16::from_be_bytes(raw[4..6].try_into().ok()?);
            other(ty, format!("({block},{offset})"))
        }
        oid::JSONPATH => {
            let (version, body) = raw.split_first()?;
            if *version != 1 {
                return None;
            }
            other(ty, String::from_utf8(body.to_vec()).ok()?)
        }
        oid::RECORD => other(ty, decode_composite(None, raw)?),
        _ if ty.name() == "citext" => text_value(raw, limit),
        _ => return None,
    })
}

fn other(ty: &Type, text: String) -> Value {
    Value::Other {
        type_name: type_name(ty),
        text,
    }
}

fn other_named(type_name: &str, text: &str) -> Value {
    Value::Other {
        type_name: type_name.to_owned(),
        text: text.to_owned(),
    }
}

/// Text when the bytes are plain UTF-8 without control characters, else hex.
fn fallback(ty: &Type, raw: &[u8]) -> Value {
    let printable = std::str::from_utf8(raw).ok().filter(|s| {
        s.chars()
            .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
    });
    let text = match printable {
        Some(s) => s.to_owned(),
        None => {
            let mut out = String::with_capacity(2 + raw.len() * 2);
            out.push_str("0x");
            for b in raw {
                out.push_str(&format!("{b:02X}"));
            }
            out
        }
    };
    other(ty, text)
}

/// The shortest decimal that round-trips a `float4`, as a `f64` (so `0.1f32`
/// shows as `0.1`, not `0.10000000149011612`).
fn shortest_f32(f: f32) -> f64 {
    if f.is_finite() {
        f.to_string().parse().unwrap_or_else(|_| f64::from(f))
    } else {
        f64::from(f)
    }
}

fn utf8_cut(raw: &[u8], limit: usize) -> usize {
    let mut cut = limit.min(raw.len());
    while cut > 0 && cut < raw.len() && (raw[cut] & 0xC0) == 0x80 {
        cut -= 1;
    }
    cut
}

fn truncated(prefix: Value, full_len: usize) -> Value {
    Value::Truncated {
        prefix: Box::new(prefix),
        full_len: Some(full_len as u64),
    }
}

fn text_value(raw: &[u8], limit: usize) -> Value {
    if raw.len() > limit {
        let cut = utf8_cut(raw, limit);
        let prefix = String::from_utf8_lossy(&raw[..cut]).into_owned();
        return truncated(Value::Text(prefix), raw.len());
    }
    Value::Text(String::from_utf8_lossy(raw).into_owned())
}

fn json_value(raw: &[u8], limit: usize) -> Value {
    if raw.len() > limit {
        let cut = utf8_cut(raw, limit);
        let prefix = String::from_utf8_lossy(&raw[..cut]).into_owned();
        return truncated(Value::Json(prefix), raw.len());
    }
    Value::Json(String::from_utf8_lossy(raw).into_owned())
}

fn bytes_value(raw: &[u8], limit: usize) -> Value {
    if raw.len() > limit {
        return truncated(Value::Bytes(raw[..limit].to_vec()), raw.len());
    }
    Value::Bytes(raw.to_vec())
}

fn uuid_text(raw: &[u8]) -> Option<String> {
    if raw.len() != 16 {
        return None;
    }
    let mut out = String::with_capacity(36);
    for (i, b) in raw.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push_str(&format!("{b:02x}"));
    }
    Some(out)
}

fn timestamp(micros: i64) -> Option<NaiveDateTime> {
    pg_epoch().checked_add_signed(TimeDelta::microseconds(micros))
}

fn time_of_day(micros: i64) -> Option<NaiveTime> {
    if !(0..86_400_000_000).contains(&micros) {
        return None;
    }
    let secs = u32::try_from(micros / 1_000_000).ok()?;
    let nanos = u32::try_from(micros % 1_000_000).ok()? * 1000;
    NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
}

fn format_time_micros(micros: i64) -> String {
    let hours = micros / 3_600_000_000;
    let minutes = (micros / 60_000_000) % 60;
    let seconds = (micros / 1_000_000) % 60;
    let frac = micros % 1_000_000;
    let mut out = format!("{hours:02}:{minutes:02}:{seconds:02}");
    if frac != 0 {
        out.push('.');
        out.push_str(format!("{frac:06}").trim_end_matches('0'));
    }
    out
}

/// `timetz` text: time of day plus the UTC offset. The wire zone counts
/// seconds west of Greenwich, so the printed offset has the opposite sign.
fn format_timetz(micros: i64, west_seconds: i32) -> String {
    let mut out = format_time_micros(micros);
    let east = -i64::from(west_seconds);
    out.push(if east < 0 { '-' } else { '+' });
    let abs = east.unsigned_abs();
    let (h, m, s) = (abs / 3600, (abs % 3600) / 60, abs % 60);
    out.push_str(&format!("{h:02}"));
    if m != 0 || s != 0 {
        out.push_str(&format!(":{m:02}"));
    }
    if s != 0 {
        out.push_str(&format!(":{s:02}"));
    }
    out
}

fn inet_text(raw: &[u8]) -> Option<String> {
    if raw.len() < 4 {
        return None;
    }
    let (family, bits, is_cidr, n) = (raw[0], raw[1], raw[2] != 0, usize::from(raw[3]));
    let addr = raw.get(4..4 + n)?;
    let (text, max_bits) = match (family, n) {
        (2, 4) => (
            Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]).to_string(),
            32,
        ),
        (3, 16) => {
            let octets: [u8; 16] = addr.try_into().ok()?;
            (Ipv6Addr::from(octets).to_string(), 128)
        }
        _ => return None,
    };
    if is_cidr || bits != max_bits {
        Some(format!("{text}/{bits}"))
    } else {
        Some(text)
    }
}

fn mac_text(raw: &[u8], len: usize) -> Option<String> {
    if raw.len() != len {
        return None;
    }
    Some(
        raw.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

fn bit_text(raw: &[u8]) -> Option<String> {
    let nbits = usize::try_from(i32::from_be_bytes(raw.get(0..4)?.try_into().ok()?)).ok()?;
    let data = raw.get(4..)?;
    if data.len() != nbits.div_ceil(8) {
        return None;
    }
    Some(
        (0..nbits)
            .map(|i| {
                if data[i / 8] & (0x80 >> (i % 8)) != 0 {
                    '1'
                } else {
                    '0'
                }
            })
            .collect(),
    )
}

/// `money` is a count of the smallest currency unit. The number of fraction
/// digits depends on `lc_monetary`; two is assumed.
fn money_text(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    format!(
        "{}{}.{:02}",
        if cents < 0 { "-" } else { "" },
        abs / 100,
        abs % 100
    )
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn decode_array(elem: &Type, raw: &[u8], limit: usize) -> Option<Value> {
    let mut r = Reader(raw);
    let ndim = r.i32()?;
    let _has_nulls = r.i32()?;
    let _elem_oid = r.u32()?;
    if !(0..=6).contains(&ndim) {
        return None;
    }
    if ndim == 0 {
        return Some(Value::Array(Vec::new()));
    }
    let mut dims = Vec::with_capacity(ndim as usize);
    for _ in 0..ndim {
        let len = usize::try_from(r.i32()?).ok()?;
        let _lower_bound = r.i32()?;
        dims.push(len);
    }
    let total = dims.iter().try_fold(1usize, |acc, &d| acc.checked_mul(d))?;
    // Every element needs at least its 4-byte length.
    if total > raw.len() / 4 {
        return None;
    }
    let mut items = Vec::with_capacity(total);
    for _ in 0..total {
        let len = r.i32()?;
        if len < 0 {
            items.push(Value::Null);
        } else {
            let bytes = r.take(usize::try_from(len).ok()?)?;
            items.push(decode_cell(elem, Some(bytes), limit));
        }
    }
    if !r.is_empty() {
        return None;
    }
    Some(nest(items, &dims))
}

/// Group a flat, row-major element list into nested arrays by dimension.
fn nest(items: Vec<Value>, dims: &[usize]) -> Value {
    let mut level = items;
    for &d in dims[1..].iter().rev() {
        let mut next = Vec::with_capacity(level.len() / d.max(1));
        let mut it = level.into_iter();
        loop {
            let chunk: Vec<Value> = it.by_ref().take(d).collect();
            if chunk.is_empty() {
                break;
            }
            next.push(Value::Array(chunk));
        }
        level = next;
    }
    Value::Array(level)
}

const RANGE_EMPTY: u8 = 0x01;
const RANGE_LB_INC: u8 = 0x02;
const RANGE_UB_INC: u8 = 0x04;
const RANGE_LB_INF: u8 = 0x08;
const RANGE_UB_INF: u8 = 0x10;

fn decode_range(sub: &Type, raw: &[u8]) -> Option<String> {
    let mut r = Reader(raw);
    let (text, _) = range_from_reader(sub, &mut r)?;
    r.is_empty().then_some(text)
}

fn range_from_reader(sub: &Type, r: &mut Reader<'_>) -> Option<(String, ())> {
    let flags = r.u8()?;
    if flags & RANGE_EMPTY != 0 {
        return Some(("empty".to_owned(), ()));
    }
    let mut bound = |infinite: bool| -> Option<String> {
        if infinite {
            return Some(String::new());
        }
        let len = usize::try_from(r.i32()?).ok()?;
        let bytes = r.take(len)?;
        Some(quote_member(&field_text(&decode_cell(
            sub,
            Some(bytes),
            usize::MAX,
        ))?))
    };
    let lower = bound(flags & RANGE_LB_INF != 0)?;
    let upper = bound(flags & RANGE_UB_INF != 0)?;
    let open = if flags & RANGE_LB_INC != 0 { '[' } else { '(' };
    let close = if flags & RANGE_UB_INC != 0 { ']' } else { ')' };
    Some((format!("{open}{lower},{upper}{close}"), ()))
}

fn decode_multirange(sub: &Type, raw: &[u8]) -> Option<String> {
    let mut r = Reader(raw);
    let count = usize::try_from(r.i32()?).ok()?;
    let mut parts = Vec::new();
    for _ in 0..count {
        let len = usize::try_from(r.i32()?).ok()?;
        let mut inner = Reader(r.take(len)?);
        parts.push(range_from_reader(sub, &mut inner)?.0);
    }
    r.is_empty().then(|| format!("{{{}}}", parts.join(",")))
}

/// Composite and `record` values in PostgreSQL's text form: `(1,"a b",)`.
fn decode_composite(types: Option<&[Type]>, raw: &[u8]) -> Option<String> {
    let mut r = Reader(raw);
    let count = usize::try_from(r.i32()?).ok()?;
    let mut parts = Vec::with_capacity(count);
    for i in 0..count {
        let field_oid = r.u32()?;
        let len = r.i32()?;
        if len < 0 {
            parts.push(String::new());
            continue;
        }
        let bytes = r.take(usize::try_from(len).ok()?)?;
        let ty = match types.and_then(|t| t.get(i)) {
            Some(t) => Some(t.clone()),
            None => Type::from_oid(field_oid),
        };
        let text = match ty {
            Some(ty) => field_text(&decode_cell(&ty, Some(bytes), usize::MAX))?,
            None => fallback_text(bytes),
        };
        parts.push(quote_member(&text));
    }
    r.is_empty().then(|| format!("({})", parts.join(",")))
}

fn fallback_text(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) if s.chars().all(|c| !c.is_control()) => s.to_owned(),
        _ => {
            let mut out = String::from("\\x");
            for b in raw {
                out.push_str(&format!("{b:02x}"));
            }
            out
        }
    }
}

/// Text of a value as PostgreSQL prints it inside composites, ranges and
/// arrays. `None` for values without a text form here.
fn field_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => (if *b { "t" } else { "f" }).to_owned(),
        Value::TimestampTz(t) => format!("{}+00", t.naive_utc().format("%Y-%m-%d %H:%M:%S%.f")),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|i| field_text(i).map_or_else(|| "NULL".to_owned(), |t| quote_member(&t)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        other => other.to_string(),
    })
}

/// Quote a composite/range member the way PostgreSQL does: when empty or when
/// it contains quotes, backslashes, brackets, commas or whitespace.
fn quote_member(s: &str) -> String {
    let needs = s.is_empty()
        || s.chars().any(|c| {
            matches!(c, '"' | '\\' | '(' | ')' | '[' | ']' | '{' | '}' | ',') || c.is_whitespace()
        });
    if !needs {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push(c);
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use tokio_postgres::types::Field;

    use super::*;

    fn dec(ty: &Type, raw: &[u8]) -> Value {
        decode_cell(ty, Some(raw), usize::MAX)
    }

    #[test]
    fn scalars() {
        assert_eq!(dec(&Type::BOOL, &[1]), Value::Bool(true));
        assert_eq!(dec(&Type::BOOL, &[0]), Value::Bool(false));
        assert_eq!(dec(&Type::INT2, &(-5i16).to_be_bytes()), Value::Int(-5));
        assert_eq!(
            dec(&Type::INT4, &i32::MIN.to_be_bytes()),
            Value::Int(i64::from(i32::MIN))
        );
        assert_eq!(
            dec(&Type::INT8, &i64::MAX.to_be_bytes()),
            Value::Int(i64::MAX)
        );
        assert_eq!(
            dec(&Type::OID, &u32::MAX.to_be_bytes()),
            Value::Int(i64::from(u32::MAX))
        );
        assert_eq!(dec(&Type::FLOAT8, &1.5f64.to_be_bytes()), Value::Float(1.5));
        assert_eq!(
            dec(&Type::TEXT, "héllo".as_bytes()),
            Value::Text("héllo".into())
        );
        assert_eq!(dec(&Type::VARCHAR, b""), Value::Text(String::new()));
        assert_eq!(dec(&Type::BYTEA, &[0, 255]), Value::Bytes(vec![0, 255]));
        assert_eq!(
            dec(&Type::XID8, &u64::MAX.to_be_bytes()),
            Value::Decimal(u64::MAX.to_string())
        );
    }

    #[test]
    fn float4_keeps_its_short_form() {
        assert_eq!(dec(&Type::FLOAT4, &0.1f32.to_be_bytes()), Value::Float(0.1));
        assert_eq!(dec(&Type::FLOAT4, &3.5f32.to_be_bytes()), Value::Float(3.5));
        match dec(&Type::FLOAT4, &f32::NAN.to_be_bytes()) {
            Value::Float(f) => assert!(f.is_nan()),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            dec(&Type::FLOAT8, &f64::NEG_INFINITY.to_be_bytes()),
            Value::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn numeric_values() {
        let wire = numeric::tests::encode("123456789012345678901234567890.12345678");
        assert_eq!(
            dec(&Type::NUMERIC, &wire),
            Value::Decimal("123456789012345678901234567890.12345678".into())
        );
        // NaN
        let mut nan = vec![0u8; 8];
        nan[4] = 0xC0;
        assert_eq!(
            dec(&Type::NUMERIC, &nan),
            Value::Other {
                type_name: "numeric".into(),
                text: "NaN".into()
            }
        );
    }

    #[test]
    fn uuid_and_json() {
        let raw = [
            0xa0, 0xee, 0xbc, 0x99, 0x9c, 0x0b, 0x4e, 0xf8, 0xbb, 0x6d, 0x6b, 0xb9, 0xbd, 0x38,
            0x0a, 0x11,
        ];
        assert_eq!(
            dec(&Type::UUID, &raw),
            Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
        );
        assert_eq!(
            dec(&Type::JSON, br#"{"a": 1}"#),
            Value::Json(r#"{"a": 1}"#.into())
        );
        let mut jsonb = vec![1u8];
        jsonb.extend(br#"{"a": 1}"#);
        assert_eq!(dec(&Type::JSONB, &jsonb), Value::Json(r#"{"a": 1}"#.into()));
        // an unknown jsonb version cannot be decoded and falls back
        let mut bad = vec![2u8];
        bad.extend(b"{}");
        assert!(matches!(dec(&Type::JSONB, &bad), Value::Other { .. }));
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            dec(&Type::DATE, &0i32.to_be_bytes()),
            Value::Date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        );
        assert_eq!(
            dec(&Type::DATE, &(-1i32).to_be_bytes()),
            Value::Date(NaiveDate::from_ymd_opt(1999, 12, 31).unwrap())
        );
        assert_eq!(
            dec(&Type::DATE, &i32::MAX.to_be_bytes()),
            Value::Other {
                type_name: "date".into(),
                text: "infinity".into()
            }
        );
        assert_eq!(
            dec(&Type::DATE, &i32::MIN.to_be_bytes()),
            Value::Other {
                type_name: "date".into(),
                text: "-infinity".into()
            }
        );

        let micros: i64 = (86_400 + 3 * 3600 + 4 * 60 + 5) * 1_000_000 + 123_456;
        let ts = NaiveDate::from_ymd_opt(2000, 1, 2)
            .unwrap()
            .and_hms_micro_opt(3, 4, 5, 123_456)
            .unwrap();
        assert_eq!(
            dec(&Type::TIMESTAMP, &micros.to_be_bytes()),
            Value::Timestamp(ts)
        );
        match dec(&Type::TIMESTAMPTZ, &micros.to_be_bytes()) {
            Value::TimestampTz(t) => {
                assert_eq!(t.naive_utc(), ts);
                assert_eq!(t.offset().local_minus_utc(), 0);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            dec(&Type::TIMESTAMP, &i64::MAX.to_be_bytes()),
            Value::Other {
                type_name: "timestamp".into(),
                text: "infinity".into()
            }
        );
        assert_eq!(
            dec(&Type::TIMESTAMPTZ, &i64::MIN.to_be_bytes()),
            Value::Other {
                type_name: "timestamptz".into(),
                text: "-infinity".into()
            }
        );
        // far outside chrono's range: falls back instead of failing
        assert!(matches!(
            dec(&Type::TIMESTAMP, &(i64::MAX - 1).to_be_bytes()),
            Value::Other { .. }
        ));

        assert_eq!(
            dec(
                &Type::TIME,
                &((3 * 3600 + 4 * 60 + 5) * 1_000_000i64 + 500_000).to_be_bytes()
            ),
            Value::Time(NaiveTime::from_hms_micro_opt(3, 4, 5, 500_000).unwrap())
        );
        assert_eq!(
            dec(&Type::TIME, &86_400_000_000i64.to_be_bytes()),
            Value::Other {
                type_name: "time".into(),
                text: "24:00:00".into()
            }
        );
    }

    #[test]
    fn timetz_text() {
        let mut raw = Vec::new();
        raw.extend(((4 * 3600 + 5 * 60 + 6) * 1_000_000i64 + 500_000).to_be_bytes());
        raw.extend((-3 * 3600i32).to_be_bytes()); // 3 hours east
        assert_eq!(
            dec(&Type::TIMETZ, &raw),
            Value::Other {
                type_name: "timetz".into(),
                text: "04:05:06.5+03".into()
            }
        );
        assert_eq!(format_timetz(0, 5 * 3600 + 30 * 60), "00:00:00-05:30");
        assert_eq!(format_timetz(0, 0), "00:00:00+00");
        assert_eq!(
            format_timetz(0, -(5 * 3600 + 45 * 60 + 30)),
            "00:00:00+05:45:30"
        );
    }

    #[test]
    fn interval_value() {
        let mut raw = Vec::new();
        raw.extend(((4 * 3600 + 5 * 60 + 6) * 1_000_000i64 + 500_000).to_be_bytes());
        raw.extend(3i32.to_be_bytes());
        raw.extend(14i32.to_be_bytes());
        assert_eq!(
            dec(&Type::INTERVAL, &raw),
            Value::Interval("1 year 2 mons 3 days 04:05:06.5".into())
        );
    }

    fn array_int4(dims: &[(i32, i32)], items: &[Option<i32>]) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend((dims.len() as i32).to_be_bytes());
        raw.extend(i32::from(items.iter().any(Option::is_none)).to_be_bytes());
        raw.extend(23u32.to_be_bytes());
        for (len, lb) in dims {
            raw.extend(len.to_be_bytes());
            raw.extend(lb.to_be_bytes());
        }
        for item in items {
            match item {
                Some(v) => {
                    raw.extend(4i32.to_be_bytes());
                    raw.extend(v.to_be_bytes());
                }
                None => raw.extend((-1i32).to_be_bytes()),
            }
        }
        raw
    }

    #[test]
    fn arrays() {
        assert_eq!(
            dec(
                &Type::INT4_ARRAY,
                &array_int4(&[(3, 1)], &[Some(1), None, Some(3)])
            ),
            Value::Array(vec![Value::Int(1), Value::Null, Value::Int(3)])
        );
        // empty array has zero dimensions
        assert_eq!(
            dec(&Type::INT4_ARRAY, &array_int4(&[], &[])),
            Value::Array(vec![])
        );
        // 2 x 3 array nests row by row
        let two_d = array_int4(
            &[(2, 1), (3, 1)],
            &[Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)],
        );
        assert_eq!(
            dec(&Type::INT4_ARRAY, &two_d),
            Value::Array(vec![
                Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
                Value::Array(vec![Value::Int(4), Value::Int(5), Value::Int(6)]),
            ])
        );
        // truncated payload falls back instead of panicking
        let mut broken = array_int4(&[(3, 1)], &[Some(1), Some(2), Some(3)]);
        broken.truncate(broken.len() - 3);
        assert!(matches!(
            dec(&Type::INT4_ARRAY, &broken),
            Value::Other { .. }
        ));
        // absurd element count
        let mut huge = array_int4(&[(1, 1)], &[Some(1)]);
        huge[12..16].copy_from_slice(&i32::MAX.to_be_bytes());
        assert!(matches!(dec(&Type::INT4_ARRAY, &huge), Value::Other { .. }));
    }

    #[test]
    fn network_types() {
        // inet 192.168.1.5 (host) and 10.0.0.0/8, cidr 10.0.0.0/8, inet6 ::1
        assert_eq!(
            dec(&Type::INET, &[2, 32, 0, 4, 192, 168, 1, 5]),
            Value::Other {
                type_name: "inet".into(),
                text: "192.168.1.5".into()
            }
        );
        assert_eq!(
            dec(&Type::INET, &[2, 24, 0, 4, 192, 168, 1, 5]),
            Value::Other {
                type_name: "inet".into(),
                text: "192.168.1.5/24".into()
            }
        );
        assert_eq!(
            dec(&Type::CIDR, &[2, 8, 1, 4, 10, 0, 0, 0]),
            Value::Other {
                type_name: "cidr".into(),
                text: "10.0.0.0/8".into()
            }
        );
        let mut v6 = vec![3, 128, 0, 16];
        v6.extend([0u8; 15]);
        v6.push(1);
        assert_eq!(
            dec(&Type::INET, &v6),
            Value::Other {
                type_name: "inet".into(),
                text: "::1".into()
            }
        );
        assert_eq!(
            dec(&Type::MACADDR, &[0x08, 0x00, 0x2b, 0x01, 0x02, 0x03]),
            Value::Other {
                type_name: "macaddr".into(),
                text: "08:00:2b:01:02:03".into()
            }
        );
    }

    #[test]
    fn bit_money_and_lsn() {
        let mut bits = 5i32.to_be_bytes().to_vec();
        bits.push(0b1011_0000);
        assert_eq!(
            dec(&Type::BIT, &bits),
            Value::Other {
                type_name: "bit".into(),
                text: "10110".into()
            }
        );
        assert_eq!(
            dec(&Type::MONEY, &123_456i64.to_be_bytes()),
            Value::Other {
                type_name: "money".into(),
                text: "1234.56".into()
            }
        );
        assert_eq!(money_text(-5), "-0.05");
        assert_eq!(
            dec(&Type::PG_LSN, &0x0000_0001_0000_00A0u64.to_be_bytes()),
            Value::Other {
                type_name: "pg_lsn".into(),
                text: "1/A0".into()
            }
        );
    }

    #[test]
    fn ranges() {
        // int4range [1,10)
        let mut raw = vec![RANGE_LB_INC];
        raw.extend(4i32.to_be_bytes());
        raw.extend(1i32.to_be_bytes());
        raw.extend(4i32.to_be_bytes());
        raw.extend(10i32.to_be_bytes());
        assert_eq!(
            dec(&Type::INT4_RANGE, &raw),
            Value::Other {
                type_name: "int4range".into(),
                text: "[1,10)".into()
            }
        );
        assert_eq!(
            dec(&Type::INT4_RANGE, &[RANGE_EMPTY]),
            Value::Other {
                type_name: "int4range".into(),
                text: "empty".into()
            }
        );
        // (,5]
        let mut open = vec![RANGE_LB_INF | RANGE_UB_INC];
        open.extend(4i32.to_be_bytes());
        open.extend(5i32.to_be_bytes());
        assert_eq!(
            dec(&Type::INT4_RANGE, &open),
            Value::Other {
                type_name: "int4range".into(),
                text: "(,5]".into()
            }
        );
    }

    #[test]
    fn composites_and_records() {
        let ty = Type::new(
            "pair".into(),
            999_001,
            Kind::Composite(vec![
                Field::new("a".into(), Type::INT4),
                Field::new("b".into(), Type::TEXT),
                Field::new("c".into(), Type::BOOL),
            ]),
            "public".into(),
        );
        let mut raw = 3i32.to_be_bytes().to_vec();
        raw.extend(23u32.to_be_bytes());
        raw.extend(4i32.to_be_bytes());
        raw.extend(7i32.to_be_bytes());
        raw.extend(25u32.to_be_bytes());
        raw.extend(3i32.to_be_bytes());
        raw.extend(b"a b");
        raw.extend(16u32.to_be_bytes());
        raw.extend((-1i32).to_be_bytes());
        assert_eq!(
            dec(&ty, &raw),
            Value::Other {
                type_name: "pair".into(),
                text: "(7,\"a b\",)".into()
            }
        );
        // record carries field OIDs on the wire
        assert_eq!(
            dec(&Type::RECORD, &raw),
            Value::Other {
                type_name: "record".into(),
                text: "(7,\"a b\",)".into()
            }
        );
    }

    #[test]
    fn enums_domains_and_unknown_types() {
        let e = Type::new(
            "mood".into(),
            999_002,
            Kind::Enum(vec!["sad".into(), "happy".into()]),
            "public".into(),
        );
        assert_eq!(dec(&e, b"happy"), Value::Text("happy".into()));
        assert_eq!(kind_of(&e), ValueKind::Text);

        let d = Type::new(
            "posint".into(),
            999_003,
            Kind::Domain(Type::INT4),
            "public".into(),
        );
        assert_eq!(dec(&d, &9i32.to_be_bytes()), Value::Int(9));
        assert_eq!(kind_of(&d), ValueKind::Int);

        let citext = Type::new("citext".into(), 999_004, Kind::Simple, "public".into());
        assert_eq!(dec(&citext, b"AbC"), Value::Text("AbC".into()));
        assert_eq!(kind_of(&citext), ValueKind::Text);

        let weird = Type::new("weird".into(), 999_005, Kind::Simple, "public".into());
        assert_eq!(
            dec(&weird, b"plain text"),
            Value::Other {
                type_name: "weird".into(),
                text: "plain text".into()
            }
        );
        assert_eq!(
            dec(&weird, &[0x00, 0x01, 0xFF]),
            Value::Other {
                type_name: "weird".into(),
                text: "0x0001FF".into()
            }
        );
        assert_eq!(kind_of(&weird), ValueKind::Other);
    }

    #[test]
    fn null_cells() {
        assert_eq!(decode_cell(&Type::INT4, None, 10), Value::Null);
    }

    #[test]
    fn long_values_are_truncated_at_character_boundaries() {
        // "é" is two bytes: cutting at 3 must not split the second é
        let text = "ééé";
        match decode_cell(&Type::TEXT, Some(text.as_bytes()), 3) {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(*prefix, Value::Text("é".into()));
                assert_eq!(full_len, Some(6));
            }
            other => panic!("{other:?}"),
        }
        match decode_cell(&Type::BYTEA, Some(&[1, 2, 3, 4]), 2) {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(*prefix, Value::Bytes(vec![1, 2]));
                assert_eq!(full_len, Some(4));
            }
            other => panic!("{other:?}"),
        }
        let mut jsonb = vec![1u8];
        jsonb.extend(br#"{"key": "value"}"#);
        match decode_cell(&Type::JSONB, Some(&jsonb), 6) {
            Value::Truncated { prefix, full_len } => {
                assert_eq!(*prefix, Value::Json(r#"{"key""#.into()));
                assert_eq!(full_len, Some(16));
            }
            other => panic!("{other:?}"),
        }
        // exactly at the limit is not truncated
        assert_eq!(
            decode_cell(&Type::TEXT, Some(b"abc"), 3),
            Value::Text("abc".into())
        );
    }

    #[test]
    fn kinds_and_names() {
        assert_eq!(kind_of(&Type::INT4), ValueKind::Int);
        assert_eq!(kind_of(&Type::NUMERIC), ValueKind::Decimal);
        assert_eq!(kind_of(&Type::TIMESTAMPTZ), ValueKind::TimestampTz);
        assert_eq!(kind_of(&Type::INT4_ARRAY), ValueKind::Array);
        assert_eq!(kind_of(&Type::JSONB), ValueKind::Json);
        assert_eq!(kind_of(&Type::INET), ValueKind::Other);
        assert_eq!(type_name(&Type::INT4_ARRAY), "int4[]");
        assert_eq!(type_name(&Type::NUMERIC), "numeric");
    }

    #[test]
    fn member_quoting_follows_postgres_rules() {
        assert_eq!(quote_member("abc"), "abc");
        assert_eq!(quote_member(""), "\"\"");
        assert_eq!(quote_member("a b"), "\"a b\"");
        assert_eq!(quote_member("a,b"), "\"a,b\"");
        assert_eq!(quote_member("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quote_member("back\\slash"), "\"back\\\\slash\"");
    }
}
