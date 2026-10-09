//! Decoder for the binary `interval` wire format and a formatter for the
//! default `IntervalStyle` (`postgres`), written from the public PostgreSQL
//! documentation (Date/Time Types, "Interval Output").
//!
//! The wire form is 16 bytes: microseconds (`i64`), days (`i32`), months
//! (`i32`), all big-endian. In the default style years and months come from the
//! month count, then days, then the time of day as `[-]HH:MM:SS[.ffffff]`.

const MICROS_PER_SECOND: i64 = 1_000_000;
const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;

/// Decode a binary `interval` into its default text form. `None` for
/// malformed input.
pub(crate) fn decode(raw: &[u8]) -> Option<String> {
    if raw.len() != 16 {
        return None;
    }
    let micros = i64::from_be_bytes(raw[0..8].try_into().ok()?);
    let days = i32::from_be_bytes(raw[8..12].try_into().ok()?);
    let months = i32::from_be_bytes(raw[12..16].try_into().ok()?);
    Some(format(months, days, micros))
}

pub(crate) fn format(months: i32, days: i32, micros: i64) -> String {
    if months == i32::MAX && days == i32::MAX && micros == i64::MAX {
        return "infinity".to_owned();
    }
    if months == i32::MIN && days == i32::MIN && micros == i64::MIN {
        return "-infinity".to_owned();
    }

    let mut out = String::new();
    let mut is_zero = true;
    let mut is_before = false;
    let mut part = |out: &mut String, value: i64, unit: &str| {
        if value == 0 {
            return;
        }
        if !is_zero {
            out.push(' ');
        }
        if is_before && value > 0 {
            out.push('+');
        }
        out.push_str(&format!("{value} {unit}"));
        if value != 1 {
            out.push('s');
        }
        // A negative field makes the sign of the next field explicit.
        is_before = value < 0;
        is_zero = false;
    };
    part(&mut out, i64::from(months / 12), "year");
    part(&mut out, i64::from(months % 12), "mon");
    part(&mut out, i64::from(days), "day");

    let hour = micros / MICROS_PER_HOUR;
    let rest = micros % MICROS_PER_HOUR;
    let minute = rest / MICROS_PER_MINUTE;
    let rest = rest % MICROS_PER_MINUTE;
    let second = rest / MICROS_PER_SECOND;
    let fraction = rest % MICROS_PER_SECOND;

    if is_zero || hour != 0 || minute != 0 || second != 0 || fraction != 0 {
        if !is_zero {
            out.push(' ');
        }
        let negative = hour < 0 || minute < 0 || second < 0 || fraction < 0;
        if negative {
            out.push('-');
        } else if is_before {
            out.push('+');
        }
        out.push_str(&format!(
            "{:02}:{:02}:{:02}",
            hour.unsigned_abs(),
            minute.unsigned_abs(),
            second.unsigned_abs()
        ));
        if fraction != 0 {
            let digits = format!("{:06}", fraction.unsigned_abs());
            out.push('.');
            out.push_str(digits.trim_end_matches('0'));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = MICROS_PER_HOUR;
    const M: i64 = MICROS_PER_MINUTE;
    const S: i64 = MICROS_PER_SECOND;

    #[test]
    fn zero_interval() {
        assert_eq!(format(0, 0, 0), "00:00:00");
    }

    #[test]
    fn mixed_fields() {
        assert_eq!(
            format(14, 3, 4 * H + 5 * M + 6 * S + 500_000),
            "1 year 2 mons 3 days 04:05:06.5"
        );
        assert_eq!(format(12, 0, 0), "1 year");
        assert_eq!(format(24, 0, 0), "2 years");
        assert_eq!(format(1, 0, 0), "1 mon");
        assert_eq!(format(0, 1, 0), "1 day");
        assert_eq!(format(0, 7, 0), "7 days");
        assert_eq!(format(0, 0, 3 * H), "03:00:00");
    }

    #[test]
    fn time_only_with_fraction() {
        assert_eq!(format(0, 0, 1), "00:00:00.000001");
        assert_eq!(format(0, 0, 10), "00:00:00.00001");
        assert_eq!(format(0, 0, 123_456), "00:00:00.123456");
        assert_eq!(format(0, 0, 90 * S), "00:01:30");
    }

    #[test]
    fn hours_beyond_a_day() {
        assert_eq!(format(0, 0, 100 * H), "100:00:00");
        assert_eq!(format(0, 0, 2_562_047_788 * H), "2562047788:00:00");
    }

    #[test]
    fn negative_fields() {
        assert_eq!(format(-14, 0, 0), "-1 years -2 mons");
        assert_eq!(format(0, -3, 0), "-3 days");
        assert_eq!(format(0, 0, -(4 * H + 5 * M + 6 * S)), "-04:05:06");
        assert_eq!(format(0, 0, -500_000), "-00:00:00.5");
        assert_eq!(format(0, -1, 0), "-1 days");
    }

    #[test]
    fn mixed_signs_get_explicit_plus() {
        assert_eq!(format(-12, 3, 0), "-1 years +3 days");
        assert_eq!(format(0, -2, 5 * H), "-2 days +05:00:00");
        assert_eq!(format(12, -1, -H), "1 year -1 days -01:00:00");
        assert_eq!(format(0, 1, -H), "1 day -01:00:00");
    }

    #[test]
    fn infinite_intervals() {
        assert_eq!(format(i32::MAX, i32::MAX, i64::MAX), "infinity");
        assert_eq!(format(i32::MIN, i32::MIN, i64::MIN), "-infinity");
    }

    #[test]
    fn extreme_finite_values_do_not_overflow() {
        let s = format(i32::MIN, i32::MIN, i64::MIN + 1);
        assert!(s.starts_with("-178956970 years -8 mons"));
        assert!(format(i32::MAX, 0, i64::MAX).contains("year"));
    }

    #[test]
    fn decodes_wire_form() {
        let mut raw = Vec::new();
        raw.extend((4 * H + 5 * M + 6 * S + 500_000).to_be_bytes());
        raw.extend(3i32.to_be_bytes());
        raw.extend(14i32.to_be_bytes());
        assert_eq!(
            decode(&raw).as_deref(),
            Some("1 year 2 mons 3 days 04:05:06.5")
        );
        assert_eq!(decode(&raw[..15]), None);
        assert_eq!(decode(&[]), None);
    }
}
