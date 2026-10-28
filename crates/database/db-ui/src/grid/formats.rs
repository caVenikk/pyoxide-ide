//! Settings > Database > Data Views (`[database.data-views]`): how the grid
//! shows dates, times and numbers. Dates use the reference's patterns
//! (`yyyy-MM-dd HH:mm:ss.SSSSSS`: the letters of Java's `SimpleDateFormat`),
//! numbers may be grouped and use another decimal separator. The formats
//! change what a cell shows, never the value: copying, exporting and the
//! value panel's text for numbers keep the exact digits.

use chrono::{Datelike as _, NaiveDate, NaiveTime, Timelike as _};
use serde::{Deserialize, Serialize};
use settings::Section;

/// The settings section's key.
pub const KEY: &str = "database.data-views";

/// The formats of the grid's values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DataFormats {
    /// A `date` column's pattern.
    pub date: String,
    /// A `time` column's pattern.
    pub time: String,
    /// A `timestamp` column's pattern.
    pub timestamp: String,
    /// A `timestamp with time zone` column's pattern (`xxx`: the offset as
    /// `+03:00`, `XXX` writes `Z` for zero).
    pub timestamp_tz: String,
    /// Group the digits of numbers (`1 234 567`).
    pub group_numbers: bool,
    /// What separates the groups.
    pub group_separator: String,
    /// What separates the fraction.
    pub decimal_separator: String,
}

impl Default for DataFormats {
    fn default() -> Self {
        Self {
            date: "yyyy-MM-dd".into(),
            time: "HH:mm:ss.SSSSSS".into(),
            timestamp: "yyyy-MM-dd HH:mm:ss.SSSSSS".into(),
            timestamp_tz: "yyyy-MM-dd HH:mm:ss.SSSSSS xxx".into(),
            group_numbers: false,
            group_separator: " ".into(),
            decimal_separator: ".".into(),
        }
    }
}

impl Section for DataFormats {
    const KEY: &'static str = KEY;
}

impl DataFormats {
    /// Why a pattern is no good, `None` when it is.
    pub fn check_pattern(pattern: &str) -> Option<String> {
        if pattern.trim().is_empty() {
            return Some("The pattern is empty".into());
        }
        let mut quoted = false;
        for c in pattern.chars() {
            if c == '\'' {
                quoted = !quoted;
            }
        }
        if quoted {
            return Some("A quote is not closed".into());
        }
        None
    }

    /// A number's text (`-1234.5`, plain notation) as the formats show it;
    /// text that is not a plain number (`NaN`, `Infinity`) stays.
    pub fn number(&self, text: &str) -> String {
        if !self.group_numbers && self.decimal_separator == "." {
            return text.to_owned();
        }
        let (sign, body) = match text.strip_prefix('-') {
            Some(rest) => ("-", rest),
            None => ("", text),
        };
        let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
        let plain = !whole.is_empty()
            && whole.chars().all(|c| c.is_ascii_digit())
            && fraction.chars().all(|c| c.is_ascii_digit());
        if !plain {
            return text.to_owned();
        }
        let mut out = String::from(sign);
        if self.group_numbers {
            let count = whole.len();
            for (i, c) in whole.chars().enumerate() {
                if i > 0 && (count - i).is_multiple_of(3) {
                    out.push_str(&self.group_separator);
                }
                out.push(c);
            }
        } else {
            out.push_str(whole);
        }
        if body.contains('.') {
            out.push_str(&self.decimal_separator);
            out.push_str(fraction);
        }
        out
    }

    pub fn format_date(&self, date: NaiveDate) -> String {
        pattern(&self.date, Some(date), None, None)
    }

    pub fn format_time(&self, time: NaiveTime) -> String {
        pattern(&self.time, None, Some(time), None)
    }

    pub fn format_timestamp(&self, at: chrono::NaiveDateTime) -> String {
        pattern(&self.timestamp, Some(at.date()), Some(at.time()), None)
    }

    pub fn format_timestamp_tz(&self, at: chrono::DateTime<chrono::FixedOffset>) -> String {
        use chrono::Offset as _;
        let local = at.naive_local();
        pattern(
            &self.timestamp_tz,
            Some(local.date()),
            Some(local.time()),
            Some(at.offset().fix().local_minus_utc()),
        )
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// `pattern` (`SimpleDateFormat` letters) for the parts given; a letter
/// whose part is missing stays out.
pub fn pattern(
    pattern: &str,
    date: Option<NaiveDate>,
    time: Option<NaiveTime>,
    offset: Option<i32>,
) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // A quoted literal; two quotes are one.
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    // Two quotes inside the literal are one.
                    if chars.get(i + 1) == Some(&'\'') {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = chars[i..].iter().take_while(|x| **x == c).count();
        i += n;
        let pad = |v: u32| format!("{v:0>n$}");
        match c {
            'y' | 'Y' | 'u' => {
                if let Some(d) = date {
                    if n == 2 {
                        out.push_str(&format!("{:02}", d.year().rem_euclid(100)));
                    } else {
                        out.push_str(&format!("{:0>n$}", d.year()));
                    }
                }
            }
            'M' | 'L' => {
                if let Some(d) = date {
                    let m = d.month0() as usize;
                    match n {
                        1 | 2 => out.push_str(&pad(d.month())),
                        3 => out.push_str(&MONTHS[m][..3]),
                        _ => out.push_str(MONTHS[m]),
                    }
                }
            }
            'd' => {
                if let Some(d) = date {
                    out.push_str(&pad(d.day()));
                }
            }
            'D' => {
                if let Some(d) = date {
                    out.push_str(&pad(d.ordinal()));
                }
            }
            'E' => {
                if let Some(d) = date {
                    let name = DAYS[d.weekday().num_days_from_monday() as usize];
                    out.push_str(if n >= 4 { name } else { &name[..3] });
                }
            }
            'H' => {
                if let Some(t) = time {
                    out.push_str(&pad(t.hour()));
                }
            }
            'k' => {
                if let Some(t) = time {
                    out.push_str(&pad(if t.hour() == 0 { 24 } else { t.hour() }));
                }
            }
            'h' => {
                if let Some(t) = time {
                    let h = t.hour() % 12;
                    out.push_str(&pad(if h == 0 { 12 } else { h }));
                }
            }
            'K' => {
                if let Some(t) = time {
                    out.push_str(&pad(t.hour() % 12));
                }
            }
            'm' => {
                if let Some(t) = time {
                    out.push_str(&pad(t.minute()));
                }
            }
            's' => {
                if let Some(t) = time {
                    out.push_str(&pad(t.second()));
                }
            }
            'S' => {
                if let Some(t) = time {
                    // Fraction digits: the leading ones of the nanoseconds
                    // (the reference's `S` is milliseconds; any count of
                    // them is the fraction cut to that many digits).
                    let nanos = format!("{:09}", t.nanosecond() % 1_000_000_000);
                    for k in 0..n {
                        out.push(nanos.as_bytes().get(k).map_or('0', |b| *b as char));
                    }
                }
            }
            'a' => {
                if let Some(t) = time {
                    out.push_str(if t.hour() < 12 { "AM" } else { "PM" });
                }
            }
            'X' | 'x' | 'Z' | 'z' => {
                if let Some(secs) = offset {
                    out.push_str(&zone(c, n, secs));
                }
            }
            other => out.extend(std::iter::repeat_n(other, n)),
        }
    }
    out
}

/// A UTC offset: `X` `+03` / `XX` `+0300` / `XXX` `+03:00` (`Z` for zero
/// with a capital `X`), `Z` and `z` `+0300`.
fn zone(letter: char, count: usize, secs: i32) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let abs = secs.unsigned_abs();
    let (h, m) = (abs / 3600, abs / 60 % 60);
    match (letter, count) {
        ('X', _) if secs == 0 => "Z".into(),
        ('X' | 'x', 1) => format!("{sign}{h:02}"),
        ('X' | 'x', 2) => format!("{sign}{h:02}{m:02}"),
        ('X' | 'x', _) => format!("{sign}{h:02}:{m:02}"),
        _ => format!("{sign}{h:02}{m:02}"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone as _};

    use super::*;

    fn at() -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 5)
            .unwrap()
            .and_hms_micro_opt(14, 7, 9, 123_456)
            .unwrap()
    }

    #[test]
    fn the_defaults_show_what_the_reference_shows() {
        let f = DataFormats::default();
        assert_eq!(f.format_timestamp(at()), "2026-09-05 14:07:09.123456");
        assert_eq!(f.format_date(at().date()), "2026-09-05");
        assert_eq!(f.format_time(at().time()), "14:07:09.123456");
        let tz = FixedOffset::east_opt(0).unwrap().from_utc_datetime(&at());
        assert_eq!(
            f.format_timestamp_tz(tz),
            "2026-09-05 14:07:09.123456 +00:00"
        );
    }

    #[test]
    fn patterns_follow_simple_date_format() {
        let d = Some(at().date());
        let t = Some(at().time());
        assert_eq!(pattern("dd.MM.yyyy", d, t, None), "05.09.2026");
        assert_eq!(pattern("d MMM yy, h:mm a", d, t, None), "5 Sep 26, 2:07 PM");
        assert_eq!(pattern("EEEE d MMMM", d, t, None), "Saturday 5 September");
        assert_eq!(pattern("HH:mm:ss.SSS", d, t, None), "14:07:09.123");
        assert_eq!(pattern("ss.SSSSSSSS", d, t, None), "09.12345600");
        assert_eq!(
            pattern("yyyy-MM-dd'T'HH:mm", d, t, None),
            "2026-09-05T14:07"
        );
        assert_eq!(pattern("'it''s' yyyy", d, t, None), "it's 2026");
        assert_eq!(pattern("HH:mm XXX", d, t, Some(3 * 3600)), "14:07 +03:00");
        assert_eq!(pattern("HH:mm XXX", d, t, Some(0)), "14:07 Z");
        assert_eq!(
            pattern("HH:mm Z", d, t, Some(-(5 * 3600 + 1800))),
            "14:07 -0530"
        );
        // A part the value lacks leaves its letters out.
        assert_eq!(pattern("yyyy HH", None, t, None), " 14");
        assert_eq!(
            DataFormats::check_pattern(""),
            Some("The pattern is empty".into())
        );
        assert!(DataFormats::check_pattern("HH 'x").is_some());
        assert!(DataFormats::check_pattern("HH:mm").is_none());
    }

    #[test]
    fn numbers_group_and_use_the_decimal_separator() {
        let mut f = DataFormats::default();
        assert_eq!(f.number("-1234567.8910"), "-1234567.8910");
        f.group_numbers = true;
        f.group_separator = ",".into();
        assert_eq!(f.number("-1234567.8910"), "-1,234,567.8910");
        assert_eq!(f.number("123"), "123");
        assert_eq!(f.number("1000"), "1,000");
        assert_eq!(f.number("NaN"), "NaN");
        f.decimal_separator = ",".into();
        f.group_separator = ".".into();
        assert_eq!(f.number("1234.5"), "1.234,5");
        // Digits beyond 64 bits stay exact.
        assert_eq!(
            f.number("123456789012345678901234567890.5"),
            "123.456.789.012.345.678.901.234.567.890,5"
        );
    }
}
