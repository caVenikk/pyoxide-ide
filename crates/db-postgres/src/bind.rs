//! Converting [`Value`] binds into PostgreSQL parameters.
//!
//! The prepared statement reports the type of every `$n`. Integers, floats
//! and booleans use the binary format when the target is the matching native
//! type. Everything else is sent in the text format, so the server parses it
//! for the actual target type (numeric, uuid, dates, enums, json, ...) and
//! reports a clear error when it does not fit.

use bytes::{BufMut, BytesMut};
use db_core::{Error, ErrorKind, Result, Value};
use tokio_postgres::types::{Format, IsNull, ToSql, Type, to_sql_checked};

/// One parameter, ready to be written into the bind message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Bound {
    Null,
    Binary(Vec<u8>),
    Text(String),
}

impl ToSql for Bound {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Bound::Null => Ok(IsNull::Yes),
            Bound::Binary(bytes) => {
                out.put_slice(bytes);
                Ok(IsNull::No)
            }
            Bound::Text(text) => {
                out.put_slice(text.as_bytes());
                Ok(IsNull::No)
            }
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    fn encode_format(&self, _ty: &Type) -> Format {
        match self {
            Bound::Text(_) => Format::Text,
            Bound::Null | Bound::Binary(_) => Format::Binary,
        }
    }

    to_sql_checked!();
}

/// Convert all binds for a statement whose parameter types are `types`.
pub(crate) fn convert_all(values: &[Value], types: &[Type]) -> Result<Vec<Bound>> {
    if values.len() != types.len() {
        return Err(Error::query(format!(
            "the statement takes {} parameter(s) but {} value(s) were bound",
            types.len(),
            values.len()
        )));
    }
    values
        .iter()
        .zip(types)
        .enumerate()
        .map(|(i, (value, ty))| convert(value, ty).map_err(|e| with_position(e, i + 1)))
        .collect()
}

fn with_position(mut e: Error, position: usize) -> Error {
    e.message = format!("parameter ${position}: {}", e.message);
    e
}

fn range_error(value: i64, ty: &Type) -> Error {
    Error::query(format!("{value} is out of range for {}", ty.name()))
}

pub(crate) fn convert(value: &Value, ty: &Type) -> Result<Bound> {
    Ok(match value {
        Value::Null => Bound::Null,
        Value::Bool(b) => {
            if *ty == Type::BOOL {
                Bound::Binary(vec![u8::from(*b)])
            } else {
                Bound::Text((if *b { "true" } else { "false" }).to_owned())
            }
        }
        Value::Int(i) => {
            if *ty == Type::INT2 {
                let v = i16::try_from(*i).map_err(|_| range_error(*i, ty))?;
                Bound::Binary(v.to_be_bytes().to_vec())
            } else if *ty == Type::INT4 {
                let v = i32::try_from(*i).map_err(|_| range_error(*i, ty))?;
                Bound::Binary(v.to_be_bytes().to_vec())
            } else if *ty == Type::INT8 {
                Bound::Binary(i.to_be_bytes().to_vec())
            } else if *ty == Type::OID {
                let v = u32::try_from(*i).map_err(|_| range_error(*i, ty))?;
                Bound::Binary(v.to_be_bytes().to_vec())
            } else {
                Bound::Text(i.to_string())
            }
        }
        Value::Float(f) => {
            if *ty == Type::FLOAT8 {
                Bound::Binary(f.to_be_bytes().to_vec())
            } else if *ty == Type::FLOAT4 {
                Bound::Binary((*f as f32).to_be_bytes().to_vec())
            } else {
                Bound::Text(float_text(*f))
            }
        }
        Value::Decimal(s)
        | Value::Text(s)
        | Value::Interval(s)
        | Value::Uuid(s)
        | Value::Json(s) => Bound::Text(s.clone()),
        Value::Other { text, .. } => Bound::Text(text.clone()),
        Value::Bytes(bytes) => {
            if *ty == Type::BYTEA {
                Bound::Binary(bytes.clone())
            } else {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "binary values can only be bound to bytea, not {}",
                        ty.name()
                    ),
                ));
            }
        }
        Value::Date(d) => Bound::Text(d.format("%Y-%m-%d").to_string()),
        Value::Time(t) => Bound::Text(t.format("%H:%M:%S%.6f").to_string()),
        Value::Timestamp(t) => Bound::Text(t.format("%Y-%m-%d %H:%M:%S%.6f").to_string()),
        Value::TimestampTz(t) => Bound::Text(t.format("%Y-%m-%d %H:%M:%S%.6f%:z").to_string()),
        Value::Array(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "array values cannot be bound; pass an array literal as text",
            ));
        }
        Value::Truncated { .. } => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "a truncated value cannot be bound",
            ));
        }
    })
}

fn float_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_owned()
    } else if f == f64::INFINITY {
        "Infinity".to_owned()
    } else if f == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        f.to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate};

    use super::*;

    fn bin(v: &[u8]) -> Bound {
        Bound::Binary(v.to_vec())
    }

    fn text(s: &str) -> Bound {
        Bound::Text(s.to_owned())
    }

    #[test]
    fn null_and_bool() {
        assert_eq!(convert(&Value::Null, &Type::INT4).unwrap(), Bound::Null);
        assert_eq!(convert(&Value::Bool(true), &Type::BOOL).unwrap(), bin(&[1]));
        assert_eq!(
            convert(&Value::Bool(false), &Type::BOOL).unwrap(),
            bin(&[0])
        );
        assert_eq!(
            convert(&Value::Bool(true), &Type::TEXT).unwrap(),
            text("true")
        );
    }

    #[test]
    fn integers_follow_the_target_type() {
        assert_eq!(
            convert(&Value::Int(7), &Type::INT2).unwrap(),
            bin(&7i16.to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Int(-7), &Type::INT4).unwrap(),
            bin(&(-7i32).to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Int(i64::MAX), &Type::INT8).unwrap(),
            bin(&i64::MAX.to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Int(4_000_000_000), &Type::OID).unwrap(),
            bin(&4_000_000_000u32.to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Int(42), &Type::NUMERIC).unwrap(),
            text("42")
        );
        assert_eq!(convert(&Value::Int(42), &Type::TEXT).unwrap(), text("42"));
        assert_eq!(convert(&Value::Int(-1), &Type::FLOAT8).unwrap(), text("-1"));
    }

    #[test]
    fn integer_range_is_checked() {
        let e = convert(&Value::Int(40_000), &Type::INT2).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Query);
        assert!(e.message.contains("out of range for int2"), "{}", e.message);
        assert!(convert(&Value::Int(i64::from(i32::MAX) + 1), &Type::INT4).is_err());
        assert!(convert(&Value::Int(-1), &Type::OID).is_err());
    }

    #[test]
    fn floats() {
        assert_eq!(
            convert(&Value::Float(1.5), &Type::FLOAT8).unwrap(),
            bin(&1.5f64.to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Float(1.5), &Type::FLOAT4).unwrap(),
            bin(&1.5f32.to_be_bytes())
        );
        assert_eq!(
            convert(&Value::Float(2.25), &Type::NUMERIC).unwrap(),
            text("2.25")
        );
        assert_eq!(
            convert(&Value::Float(f64::NAN), &Type::NUMERIC).unwrap(),
            text("NaN")
        );
        assert_eq!(
            convert(&Value::Float(f64::INFINITY), &Type::NUMERIC).unwrap(),
            text("Infinity")
        );
        assert_eq!(
            convert(&Value::Float(f64::NEG_INFINITY), &Type::NUMERIC).unwrap(),
            text("-Infinity")
        );
        assert_eq!(
            convert(&Value::Float(1e21), &Type::NUMERIC).unwrap(),
            text("1000000000000000000000")
        );
    }

    #[test]
    fn decimals_and_text_travel_as_text() {
        let big = "12345678901234567890123456789012345678.123";
        assert_eq!(
            convert(&Value::Decimal(big.into()), &Type::NUMERIC).unwrap(),
            text(big)
        );
        assert_eq!(
            convert(&Value::Text("héllo".into()), &Type::TEXT).unwrap(),
            text("héllo")
        );
        assert_eq!(
            convert(&Value::Text("héllo".into()), &Type::VARCHAR).unwrap(),
            text("héllo")
        );
        // the server parses the text for the real target type
        assert_eq!(
            convert(
                &Value::Text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into()),
                &Type::UUID
            )
            .unwrap(),
            text("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11")
        );
        assert_eq!(
            convert(&Value::Text("{}".into()), &Type::JSONB).unwrap(),
            text("{}")
        );
    }

    #[test]
    fn bytes_only_fit_bytea() {
        assert_eq!(
            convert(&Value::Bytes(vec![0, 255]), &Type::BYTEA).unwrap(),
            bin(&[0, 255])
        );
        let e = convert(&Value::Bytes(vec![1]), &Type::TEXT).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
    }

    #[test]
    fn temporal_values_use_iso_text() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(
            convert(&Value::Date(d), &Type::DATE).unwrap(),
            text("2026-01-02")
        );
        let ts = d.and_hms_micro_opt(3, 4, 5, 600).unwrap();
        assert_eq!(
            convert(&Value::Timestamp(ts), &Type::TIMESTAMP).unwrap(),
            text("2026-01-02 03:04:05.000600")
        );
        let tz = DateTime::parse_from_rfc3339("2026-01-02T03:04:05+03:00").unwrap();
        assert_eq!(
            convert(&Value::TimestampTz(tz), &Type::TIMESTAMPTZ).unwrap(),
            text("2026-01-02 03:04:05.000000+03:00")
        );
        assert_eq!(
            convert(&Value::Time(ts.time()), &Type::TIME).unwrap(),
            text("03:04:05.000600")
        );
        assert_eq!(
            convert(&Value::Interval("1 day".into()), &Type::INTERVAL).unwrap(),
            text("1 day")
        );
        assert_eq!(
            convert(
                &Value::Other {
                    type_name: "inet".into(),
                    text: "10.0.0.1".into()
                },
                &Type::INET
            )
            .unwrap(),
            text("10.0.0.1")
        );
    }

    #[test]
    fn unsupported_values() {
        let e = convert(&Value::Array(vec![]), &Type::INT4_ARRAY).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
        let cut = Value::Truncated {
            prefix: Box::new(Value::Text("a".into())),
            full_len: None,
        };
        assert_eq!(
            convert(&cut, &Type::TEXT).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn convert_all_checks_count_and_names_the_parameter() {
        let e = convert_all(&[Value::Int(1)], &[Type::INT4, Type::INT4]).unwrap_err();
        assert!(e.message.contains("2 parameter(s) but 1"), "{}", e.message);

        let e = convert_all(
            &[Value::Int(1), Value::Int(99_999)],
            &[Type::INT4, Type::INT2],
        )
        .unwrap_err();
        assert!(e.message.starts_with("parameter $2:"), "{}", e.message);

        let ok = convert_all(&[Value::Int(1), Value::Null], &[Type::INT4, Type::TEXT]).unwrap();
        assert_eq!(ok, vec![bin(&1i32.to_be_bytes()), Bound::Null]);
    }

    #[test]
    fn wire_format_matches_the_variant() {
        assert!(matches!(
            Bound::Text("x".into()).encode_format(&Type::TEXT),
            Format::Text
        ));
        assert!(matches!(
            Bound::Binary(vec![1]).encode_format(&Type::INT2),
            Format::Binary
        ));
        assert!(matches!(
            Bound::Null.encode_format(&Type::TEXT),
            Format::Binary
        ));
        let mut out = BytesMut::new();
        assert!(matches!(
            Bound::Null.to_sql(&Type::TEXT, &mut out).unwrap(),
            IsNull::Yes
        ));
        assert!(out.is_empty());
        assert!(matches!(
            Bound::Text("ab".into())
                .to_sql(&Type::TEXT, &mut out)
                .unwrap(),
            IsNull::No
        ));
        assert_eq!(&out[..], b"ab");
    }
}
