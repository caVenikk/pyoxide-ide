//! Pieces of a grid's value texts: the `<null>` marker, alignment,
//! PostgreSQL intervals in words (`4 mins 58 secs`) and the exact decimals
//! the aggregates use. What a cell shows for each type is [`super::value`].

use db_core::{Value, ValueKind};

/// What NULL looks like (grey, unlike an empty string).
pub const NULL_TEXT: &str = "<null>";

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

    /// The number as a float, for statistics.
    pub fn to_f64(self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }

    /// This divided by `n` (not zero), to 34 significant digits rounded
    /// half to even (`DECIMAL128`, as the reference divides), without the
    /// trailing zeros of the fraction.
    pub fn divided_by(self, n: u64) -> String {
        const DIGITS: usize = 34;
        let negative = self.mantissa < 0;
        let q = self.mantissa.unsigned_abs();
        let d = u128::from(n.max(1));
        let whole = (q / d).to_string();
        let mut rem = q % d;
        // The digits of the quotient, the point after `point` of them.
        let mut digits: Vec<u8> = if whole == "0" {
            Vec::new()
        } else {
            whole.bytes().map(|b| b - b'0').collect()
        };
        let mut point = if whole == "0" {
            0
        } else {
            whole.len() as isize
        };
        let mut significant = digits.len();
        while rem != 0 && significant < DIGITS {
            rem *= 10;
            let digit = (rem / d) as u8;
            rem %= d;
            if digits.is_empty() && digit == 0 {
                // A leading zero of the fraction: no significant digit yet.
                point -= 1;
                continue;
            }
            digits.push(digit);
            significant += 1;
        }
        if rem != 0 {
            let twice = rem * 2;
            let up = twice > d || (twice == d && digits.last().is_some_and(|x| x % 2 == 1));
            if up {
                let mut i = digits.len();
                loop {
                    if i == 0 {
                        digits.insert(0, 1);
                        point += 1;
                        break;
                    }
                    i -= 1;
                    if digits[i] == 9 {
                        digits[i] = 0;
                    } else {
                        digits[i] += 1;
                        break;
                    }
                }
            }
        }
        if digits.is_empty() {
            return "0".to_owned();
        }
        // Now shift the point left by this number's scale.
        let point = point - self.scale as isize;
        let mut out = String::new();
        if negative {
            out.push('-');
        }
        let text: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
        if point <= 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-point) as usize));
            out.push_str(&text);
        } else if point as usize >= text.len() {
            out.push_str(&text);
            out.extend(std::iter::repeat_n('0', point as usize - text.len()));
        } else {
            out.push_str(&text[..point as usize]);
            out.push('.');
            out.push_str(&text[point as usize..]);
        }
        if out.contains('.') {
            let trimmed = out.trim_end_matches('0').trim_end_matches('.');
            out = trimmed.to_owned();
        }
        out
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

    #[test]
    fn numbers_are_right_aligned() {
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
    fn averages_have_thirty_four_digits() {
        let d = |s: &str| Decimal::parse(s).unwrap();
        assert_eq!(
            d("109.626").divided_by(438),
            "0.2502876712328767123287671232876712"
        );
        assert_eq!(d("0.501").divided_by(2), "0.2505");
        assert_eq!(d("10").divided_by(4), "2.5");
        assert_eq!(
            d("-10").divided_by(3),
            "-3.333333333333333333333333333333333"
        );
        assert_eq!(d("2").divided_by(3), "0.6666666666666666666666666666666667");
        assert_eq!(d("1200").divided_by(2), "600");
        assert_eq!(d("0.0001").divided_by(1), "0.0001");
        assert_eq!(Decimal::default().divided_by(5), "0");
        assert_eq!(
            d("1").compare(d("0.999")),
            Some(std::cmp::Ordering::Greater)
        );
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
