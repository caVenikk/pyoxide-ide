//! `Value` parameters as Oracle bind variables.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, Timelike};
use db_core::{Error, Result, Value};
use oracle::sql_type::{OracleType, Timestamp, ToSql};
use oracle::{Connection, SqlValue};

use crate::values::normalize_number;

/// One bound parameter, with the Oracle type it is sent as.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Bind {
    /// A NULL; its type does not matter to the server.
    Null,
    Int(i64),
    Float(f64),
    /// Exact decimal text in plain notation. The client converts it to a
    /// NUMBER itself, independent of `NLS_NUMERIC_CHARACTERS`.
    Decimal(String),
    /// Sent as VARCHAR2, so comparisons with VARCHAR2 columns can use their
    /// indexes (an NVARCHAR2 bind would convert the column).
    Text(String),
    Bytes(Vec<u8>),
    /// A date with a time of day at whole seconds, sent as DATE.
    Date(Timestamp),
    /// A timestamp with fractional seconds, sent as TIMESTAMP.
    Timestamp(Timestamp),
    TimestampTz(Timestamp),
}

pub(crate) fn binds_from_values(params: &[Value]) -> Result<Vec<Bind>> {
    params
        .iter()
        .enumerate()
        .map(|(i, v)| {
            bind_from_value(v).map_err(|mut e| {
                e.message = format!("parameter :{}: {}", i + 1, e.message);
                e
            })
        })
        .collect()
}

pub(crate) fn bind_from_value(v: &Value) -> Result<Bind> {
    Ok(match v {
        Value::Null => Bind::Null,
        // Oracle SQL has no BOOLEAN before 23ai; 1 and 0 convert to it there.
        Value::Bool(b) => Bind::Int(i64::from(*b)),
        Value::Int(i) => Bind::Int(*i),
        Value::Float(f) => Bind::Float(*f),
        Value::Decimal(s) => Bind::Decimal(
            normalize_number(s)
                .ok_or_else(|| Error::query(format!("{s:?} is not a decimal number")))?,
        ),
        Value::Text(s) | Value::Interval(s) | Value::Uuid(s) | Value::Json(s) => {
            Bind::Text(s.clone())
        }
        Value::Other { text, .. } => Bind::Text(text.clone()),
        Value::Bytes(b) => Bind::Bytes(b.clone()),
        Value::Date(d) => Bind::Date(timestamp(&d.and_hms_opt(0, 0, 0).expect("midnight"))?),
        Value::Timestamp(t) => {
            let ts = timestamp(t)?;
            if t.nanosecond() == 0 {
                Bind::Date(ts)
            } else {
                Bind::Timestamp(ts)
            }
        }
        Value::TimestampTz(t) => Bind::TimestampTz(timestamp_tz(t)?),
        Value::Time(_) => {
            return Err(Error::unsupported(
                "Oracle has no TIME type; use a TIMESTAMP or an INTERVAL",
            ));
        }
        Value::Array(_) => {
            return Err(Error::unsupported(
                "array parameters are not supported by the Oracle Client backend",
            ));
        }
        Value::Truncated { .. } => {
            return Err(Error::unsupported(
                "a truncated value cannot be used as a parameter",
            ));
        }
    })
}

fn timestamp(dt: &NaiveDateTime) -> Result<Timestamp> {
    Timestamp::new(
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        // Leap seconds carry nanoseconds beyond 10^9.
        dt.nanosecond().min(999_999_999),
    )
    .map_err(|e| Error::query(format!("{dt} is outside the Oracle date range: {e}")))
}

fn timestamp_tz(dt: &DateTime<FixedOffset>) -> Result<Timestamp> {
    timestamp(&dt.naive_local())?
        .and_tz_offset(dt.offset().local_minus_utc())
        .map_err(|e| Error::query(format!("invalid time zone offset in {dt}: {e}")))
}

/// The Oracle type a bind is declared with.
pub(crate) fn bind_oratype(b: &Bind) -> OracleType {
    match b {
        Bind::Null => OracleType::Varchar2(1),
        Bind::Int(_) | Bind::Decimal(_) => OracleType::Number(0, 0),
        // NaN and the infinities exist only as BINARY_DOUBLE.
        Bind::Float(f) if !f.is_finite() => OracleType::BinaryDouble,
        Bind::Float(_) => OracleType::Number(0, 0),
        // The size is in characters; the client allocates the bytes.
        Bind::Text(s) => OracleType::Varchar2(clamp_size(s.chars().count())),
        Bind::Bytes(b) => OracleType::Raw(clamp_size(b.len())),
        Bind::Date(_) => OracleType::Date,
        Bind::Timestamp(_) => OracleType::Timestamp(9),
        Bind::TimestampTz(_) => OracleType::TimestampTZ(9),
    }
}

fn clamp_size(n: usize) -> u32 {
    u32::try_from(n.max(1)).unwrap_or(u32::MAX)
}

impl ToSql for Bind {
    fn oratype(&self, _conn: &Connection) -> oracle::Result<OracleType> {
        Ok(bind_oratype(self))
    }

    fn to_sql(&self, val: &mut SqlValue<'_>) -> oracle::Result<()> {
        match self {
            Bind::Null => val.set_null(),
            Bind::Int(i) => val.set(i),
            Bind::Float(f) => val.set(f),
            Bind::Decimal(s) => val.set(&s.as_str()),
            Bind::Text(s) => val.set(&s.as_str()),
            Bind::Bytes(b) => val.set(&b.as_slice()),
            Bind::Date(t) | Bind::Timestamp(t) | Bind::TimestampTz(t) => val.set(t),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime, TimeZone};
    use db_core::ErrorKind;

    use super::*;

    fn bind(v: Value) -> Bind {
        bind_from_value(&v).unwrap()
    }

    #[test]
    fn scalar_values() {
        assert_eq!(bind(Value::Null), Bind::Null);
        assert_eq!(bind(Value::Int(-5)), Bind::Int(-5));
        assert_eq!(bind(Value::Bool(true)), Bind::Int(1));
        assert_eq!(bind(Value::Bool(false)), Bind::Int(0));
        assert_eq!(bind(Value::Float(1.5)), Bind::Float(1.5));
        assert_eq!(bind(Value::Text("ёж".into())), Bind::Text("ёж".into()));
        assert_eq!(bind(Value::Bytes(vec![1, 2])), Bind::Bytes(vec![1, 2]));
        assert_eq!(bind(Value::Uuid("u".into())), Bind::Text("u".into()));
        assert_eq!(bind(Value::Json("{}".into())), Bind::Text("{}".into()));
        assert_eq!(
            bind(Value::Interval("+01-02".into())),
            Bind::Text("+01-02".into())
        );
        assert_eq!(
            bind(Value::Other {
                type_name: "ROWID".into(),
                text: "AAAB".into()
            }),
            Bind::Text("AAAB".into())
        );
    }

    #[test]
    fn decimals_are_validated_and_normalized() {
        assert_eq!(
            bind(Value::Decimal(
                "12345678901234567890123456789012345678".into()
            )),
            Bind::Decimal("12345678901234567890123456789012345678".into())
        );
        assert_eq!(
            bind(Value::Decimal(".5".into())),
            Bind::Decimal("0.5".into())
        );
        assert_eq!(bind(Value::Decimal("+7".into())), Bind::Decimal("7".into()));
        assert_eq!(
            bind(Value::Decimal("1E+3".into())),
            Bind::Decimal("1000".into())
        );
        assert_eq!(
            bind(Value::Decimal("-12.50".into())),
            Bind::Decimal("-12.5".into())
        );
        for bad in ["", "abc", "1,5", "NaN", "1 2"] {
            let e = bind_from_value(&Value::Decimal(bad.into())).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Query, "{bad:?}");
        }
    }

    #[test]
    fn dates_and_timestamps() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        match bind(Value::Date(d)) {
            Bind::Date(ts) => {
                assert_eq!(
                    (ts.year(), ts.month(), ts.day(), ts.hour()),
                    (2026, 1, 2, 0)
                );
            }
            other => panic!("{other:?}"),
        }
        // Whole seconds go as DATE, so DATE columns compare without a conversion.
        let whole = d.and_hms_opt(3, 4, 5).unwrap();
        assert!(matches!(bind(Value::Timestamp(whole)), Bind::Date(_)));
        let frac = d.and_hms_micro_opt(3, 4, 5, 600).unwrap();
        match bind(Value::Timestamp(frac)) {
            Bind::Timestamp(ts) => assert_eq!(ts.nanosecond(), 600_000),
            other => panic!("{other:?}"),
        }
        let tz = FixedOffset::east_opt(3 * 3600 + 1800)
            .unwrap()
            .from_local_datetime(&frac)
            .unwrap();
        match bind(Value::TimestampTz(tz)) {
            Bind::TimestampTz(ts) => {
                assert_eq!(ts.tz_hour_offset(), 3);
                assert_eq!(ts.tz_minute_offset(), 30);
                assert_eq!(ts.hour(), 3);
            }
            other => panic!("{other:?}"),
        }
        let west = FixedOffset::west_opt(5 * 3600 + 1800)
            .unwrap()
            .from_local_datetime(&frac)
            .unwrap();
        match bind(Value::TimestampTz(west)) {
            Bind::TimestampTz(ts) => {
                assert_eq!(ts.tz_hour_offset(), -5);
                assert_eq!(ts.tz_minute_offset(), -30);
            }
            other => panic!("{other:?}"),
        }
        // A leap second does not overflow the nanosecond field.
        let leap = d.and_hms_nano_opt(23, 59, 59, 1_500_000_000).unwrap();
        assert!(matches!(bind(Value::Timestamp(leap)), Bind::Timestamp(_)));
    }

    #[test]
    fn dates_outside_the_oracle_range_are_errors() {
        let far = NaiveDate::from_ymd_opt(20000, 1, 1).unwrap();
        let e = bind_from_value(&Value::Date(far)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query);
    }

    #[test]
    fn unsupported_values() {
        for v in [
            Value::Time(NaiveTime::from_hms_opt(1, 2, 3).unwrap()),
            Value::Array(vec![Value::Int(1)]),
            Value::Truncated {
                prefix: Box::new(Value::Text("a".into())),
                full_len: Some(2),
            },
        ] {
            assert_eq!(
                bind_from_value(&v).unwrap_err().kind,
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn errors_name_the_parameter() {
        let e = binds_from_values(&[Value::Int(1), Value::Decimal("x".into())]).unwrap_err();
        assert!(e.message.starts_with("parameter :2:"), "{}", e.message);
    }

    #[test]
    fn declared_types() {
        assert_eq!(bind_oratype(&Bind::Null), OracleType::Varchar2(1));
        assert_eq!(bind_oratype(&Bind::Int(1)), OracleType::Number(0, 0));
        assert_eq!(
            bind_oratype(&Bind::Decimal("1".into())),
            OracleType::Number(0, 0)
        );
        assert_eq!(bind_oratype(&Bind::Float(0.5)), OracleType::Number(0, 0));
        assert_eq!(
            bind_oratype(&Bind::Float(f64::NAN)),
            OracleType::BinaryDouble
        );
        assert_eq!(
            bind_oratype(&Bind::Float(f64::INFINITY)),
            OracleType::BinaryDouble
        );
        // The size counts characters, not bytes, and is never zero.
        assert_eq!(
            bind_oratype(&Bind::Text("ёжик".into())),
            OracleType::Varchar2(4)
        );
        assert_eq!(
            bind_oratype(&Bind::Text(String::new())),
            OracleType::Varchar2(1)
        );
        assert_eq!(bind_oratype(&Bind::Bytes(vec![0; 10])), OracleType::Raw(10));
        assert_eq!(bind_oratype(&Bind::Bytes(vec![])), OracleType::Raw(1));
        let ts = Timestamp::new(2026, 1, 1, 0, 0, 0, 0).unwrap();
        assert_eq!(bind_oratype(&Bind::Date(ts)), OracleType::Date);
        assert_eq!(bind_oratype(&Bind::Timestamp(ts)), OracleType::Timestamp(9));
        assert_eq!(
            bind_oratype(&Bind::TimestampTz(ts)),
            OracleType::TimestampTZ(9)
        );
    }
}
