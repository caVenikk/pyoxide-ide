//! The text a grid cell shows for a value, as the reference shows it by
//! default: `<null>` for SQL NULL, timestamps with six fraction digits,
//! PostgreSQL intervals in words (`4 mins 58 secs`), numbers in their
//! shortest exact form. Display types and formats are the value panel's
//! (#306); this is the default text only.

use db_core::{Value, ValueKind};

/// What NULL looks like (grey, unlike an empty string).
pub const NULL_TEXT: &str = "<null>";

/// The cell text of a value; `None` for SQL NULL.
pub fn cell_text(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => return None,
        Value::Timestamp(t) => t.format("%Y-%m-%d %H:%M:%S%.6f").to_string(),
        Value::TimestampTz(t) => t.format("%Y-%m-%d %H:%M:%S%.6f %:z").to_string(),
        Value::Interval(text) => interval_words(text).unwrap_or_else(|| text.clone()),
        Value::Truncated { prefix, .. } => cell_text(prefix).unwrap_or_default(),
        other => other.to_string(),
    })
}

/// Numbers are right-aligned.
pub fn right_aligned(kind: ValueKind) -> bool {
    matches!(kind, ValueKind::Int | ValueKind::Float | ValueKind::Decimal)
}

/// A PostgreSQL interval in its default output style
/// (`1 year 2 mons 3 days 04:05:06.5`, `-00:00:01`) in the words the
/// reference shows: `1 years 2 mons 3 days 4 hours 5 mins 6.5 secs`, the
/// parts that are zero left out (`0 secs` when all are). `None` for text
/// in another form.
pub fn interval_words(text: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut words = text.split_whitespace().peekable();
    let mut seen_time = false;
    while let Some(word) = words.next() {
        if word.contains(':') {
            if seen_time {
                return None;
            }
            seen_time = true;
            let (negative, clock) = match word.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, word.strip_prefix('+').unwrap_or(word)),
            };
            let mut fields = clock.split(':');
            let hours: u64 = fields.next()?.parse().ok()?;
            let minutes: u64 = fields.next()?.parse().ok()?;
            let seconds = fields.next()?;
            if fields.next().is_some() {
                return None;
            }
            let seconds = trim_fraction(seconds)?;
            let sign = if negative { "-" } else { "" };
            for (n, unit) in [(hours, "hours"), (minutes, "mins")] {
                if n != 0 {
                    parts.push(format!("{sign}{n} {unit}"));
                }
            }
            if seconds != "0" {
                parts.push(format!("{sign}{seconds} secs"));
            }
            continue;
        }
        let n: i64 = word.parse().ok()?;
        let unit = words.next()?;
        let unit = match unit.trim_end_matches('s') {
            "year" => "years",
            "mon" => "mons",
            "day" => "days",
            _ => return None,
        };
        if n != 0 {
            parts.push(format!("{n} {unit}"));
        }
    }
    if parts.is_empty() {
        return Some("0 secs".to_owned());
    }
    Some(parts.join(" "))
}

/// `05` -> `5`, `06.500` -> `6.5`, `00` -> `0`.
fn trim_fraction(seconds: &str) -> Option<String> {
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let whole: u64 = whole.parse().ok()?;
    if !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let fraction = fraction.trim_end_matches('0');
    Some(if fraction.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    })
}

/// An exact decimal for sums of selections: `mantissa / 10^scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decimal {
    mantissa: i128,
    scale: u32,
}

impl Decimal {
    const MAX_SCALE: u32 = 20;

    /// The number a value holds, when it is one.
    pub fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Int(n) => Some(Self {
                mantissa: i128::from(*n),
                scale: 0,
            }),
            Value::Float(f) if f.is_finite() => Self::parse(&f.to_string()),
            Value::Decimal(s) => Self::parse(s),
            Value::Truncated { prefix, .. } => Self::of(prefix),
            _ => None,
        }
    }

    /// Plain notation: `-12.50`, `3`, `1e-7` (Rust's float text).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (body, exponent) = match text.split_once(['e', 'E']) {
            Some((b, e)) => (b, e.parse::<i32>().ok()?),
            None => (text, 0),
        };
        let (negative, body) = match body.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, body.strip_prefix('+').unwrap_or(body)),
        };
        let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        let digits = format!("{whole}{fraction}");
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut mantissa: i128 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        let mut scale = fraction.len() as i32 - exponent;
        while scale < 0 {
            mantissa = mantissa.checked_mul(10)?;
            scale += 1;
        }
        let mut d = Self {
            mantissa: if negative { -mantissa } else { mantissa },
            scale: scale as u32,
        };
        while d.scale > Self::MAX_SCALE {
            d.mantissa /= 10;
            d.scale -= 1;
        }
        Some(d)
    }

    fn rescale(self, scale: u32) -> Option<i128> {
        let mut m = self.mantissa;
        for _ in self.scale..scale {
            m = m.checked_mul(10)?;
        }
        Some(m)
    }

    /// The order of two numbers (`None` when one is too large to bring to
    /// the same scale).
    pub fn compare(self, other: Self) -> Option<std::cmp::Ordering> {
        let scale = self.scale.max(other.scale);
        Some(self.rescale(scale)?.cmp(&other.rescale(scale)?))
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        let scale = self.scale.max(other.scale);
        Some(Self {
            mantissa: self.rescale(scale)?.checked_add(other.rescale(scale)?)?,
            scale,
        })
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let negative = self.mantissa < 0;
        let digits = self.mantissa.unsigned_abs().to_string();
        let scale = self.scale as usize;
        let (whole, fraction) = if digits.len() > scale {
            let (w, fr) = digits.split_at(digits.len() - scale);
            (w.to_owned(), fr.to_owned())
        } else {
            ("0".to_owned(), format!("{digits:0>scale$}"))
        };
        let fraction = fraction.trim_end_matches('0');
        if negative && (whole != "0" || !fraction.is_empty()) {
            f.write_str("-")?;
        }
        f.write_str(&whole)?;
        if !fraction.is_empty() {
            write!(f, ".{fraction}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, TimeZone as _};

    #[test]
    fn values_show_as_in_the_reference() {
        assert_eq!(cell_text(&Value::Null), None);
        let t = NaiveDate::from_ymd_opt(2026, 9, 25)
            .unwrap()
            .and_hms_opt(23, 54, 0)
            .unwrap();
        let tz = FixedOffset::east_opt(0).unwrap().from_utc_datetime(&t);
        assert_eq!(
            cell_text(&Value::TimestampTz(tz)).unwrap(),
            "2026-09-25 23:54:00.000000 +00:00"
        );
        assert_eq!(
            cell_text(&Value::Timestamp(t)).unwrap(),
            "2026-09-25 23:54:00.000000"
        );
        assert_eq!(cell_text(&Value::Float(0.998)).unwrap(), "0.998");
        assert_eq!(cell_text(&Value::Float(0.5)).unwrap(), "0.5");
        assert_eq!(cell_text(&Value::Bool(true)).unwrap(), "true");
        assert!(right_aligned(ValueKind::Float));
        assert!(!right_aligned(ValueKind::Text));
    }

    #[test]
    fn intervals_in_words() {
        assert_eq!(interval_words("00:04:58").unwrap(), "4 mins 58 secs");
        assert_eq!(interval_words("00:08:00").unwrap(), "8 mins");
        assert_eq!(interval_words("00:08:01").unwrap(), "8 mins 1 secs");
        assert_eq!(interval_words("00:00:01").unwrap(), "1 secs");
        assert_eq!(
            interval_words("1 year 2 mons 3 days 04:05:06.5").unwrap(),
            "1 years 2 mons 3 days 4 hours 5 mins 6.5 secs"
        );
        assert_eq!(interval_words("-00:00:01").unwrap(), "-1 secs");
        assert_eq!(interval_words("00:00:00").unwrap(), "0 secs");
        assert_eq!(interval_words("P1D"), None);
    }

    #[test]
    fn exact_sums() {
        let sum = [0.001, 0.002, 0.25, 109.373]
            .iter()
            .map(|f| Decimal::of(&Value::Float(*f)).unwrap())
            .fold(Decimal::default(), |a, b| a.checked_add(b).unwrap());
        assert_eq!(sum.to_string(), "109.626");
        assert_eq!(Decimal::parse("-12.50").unwrap().to_string(), "-12.5");
        assert_eq!(Decimal::parse("1e-7").unwrap().to_string(), "0.0000001");
        assert_eq!(Decimal::parse("12e3").unwrap().to_string(), "12000");
        assert_eq!(Decimal::default().to_string(), "0");
        assert_eq!(Decimal::of(&Value::Text("1".into())), None);
    }
}
