//! Helpers for drivers that build [`Value`]s: the size limit of
//! `ExecOptions::max_value_bytes` and exact decimal text.

use crate::Value;

/// The largest index `<= max` that is a character boundary of `s`.
pub fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// The length of the longest prefix of `raw` with at most `max` bytes that
/// does not end inside a UTF-8 sequence. For text that is not valid UTF-8
/// it still never splits a sequence that starts before `max`.
pub fn utf8_prefix_len(raw: &[u8], max: usize) -> usize {
    let mut cut = max.min(raw.len());
    while cut > 0 && cut < raw.len() && (raw[cut] & 0xC0) == 0x80 {
        cut -= 1;
    }
    cut
}

/// A [`Value::Truncated`] with a known full length (in the backend's unit,
/// see [`Value::Truncated`]).
pub fn truncated(prefix: Value, full_len: u64) -> Value {
    Value::Truncated {
        prefix: Box::new(prefix),
        full_len: Some(full_len),
    }
}

/// Cut `s` to `max` bytes on a character boundary; `None` when it fits.
fn cut_string(mut s: String, max: usize) -> Result<String, (String, u64)> {
    if s.len() <= max {
        return Ok(s);
    }
    let full = s.len() as u64;
    s.truncate(floor_char_boundary(&s, max));
    Err((s, full))
}

/// `Text`, or `Truncated` with a `Text` prefix of at most `max` bytes (cut
/// on a character boundary) and the full length in bytes.
pub fn truncate_text(s: String, max: usize) -> Value {
    match cut_string(s, max) {
        Ok(s) => Value::Text(s),
        Err((prefix, full)) => truncated(Value::Text(prefix), full),
    }
}

/// [`truncate_text`] for a JSON document: `Json`, or `Truncated` with a
/// `Json` prefix.
pub fn truncate_json(s: String, max: usize) -> Value {
    match cut_string(s, max) {
        Ok(s) => Value::Json(s),
        Err((prefix, full)) => truncated(Value::Json(prefix), full),
    }
}

/// [`truncate_text`] for raw bytes that should be UTF-8; invalid sequences
/// become U+FFFD. The full length is the number of raw bytes.
pub fn text_from_utf8(raw: &[u8], max: usize) -> Value {
    if raw.len() > max {
        let cut = utf8_prefix_len(raw, max);
        let prefix = String::from_utf8_lossy(&raw[..cut]).into_owned();
        return truncated(Value::Text(prefix), raw.len() as u64);
    }
    Value::Text(String::from_utf8_lossy(raw).into_owned())
}

/// [`text_from_utf8`] for a JSON document.
pub fn json_from_utf8(raw: &[u8], max: usize) -> Value {
    if raw.len() > max {
        let cut = utf8_prefix_len(raw, max);
        let prefix = String::from_utf8_lossy(&raw[..cut]).into_owned();
        return truncated(Value::Json(prefix), raw.len() as u64);
    }
    Value::Json(String::from_utf8_lossy(raw).into_owned())
}

/// `Bytes`, or `Truncated` with the first `max` bytes and the full length.
pub fn truncate_bytes(mut b: Vec<u8>, max: usize) -> Value {
    if b.len() <= max {
        return Value::Bytes(b);
    }
    let full = b.len() as u64;
    b.truncate(max);
    truncated(Value::Bytes(b), full)
}

/// [`truncate_bytes`] without taking ownership: copies at most `max` bytes.
pub fn bytes_from_slice(raw: &[u8], max: usize) -> Value {
    if raw.len() > max {
        return truncated(Value::Bytes(raw[..max].to_vec()), raw.len() as u64);
    }
    Value::Bytes(raw.to_vec())
}

/// Plain decimal notation for the text form of an exact number: no
/// exponent, a `0` before a leading point, no trailing fractional zeros, `-`
/// for negatives (never `-0`). `None` for text that is not a finite decimal
/// number.
pub fn normalize_decimal(text: &str) -> Option<String> {
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
    // Real exponents are far below this; it only bounds the expansion.
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

/// Pad the fraction of a plain decimal to `scale` digits (`5` -> `5.00`
/// for scale 2). Longer fractions are kept as they are.
pub fn pad_decimal_scale(plain: &str, scale: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(v: Value) -> (Value, Option<u64>) {
        match v {
            Value::Truncated { prefix, full_len } => (*prefix, full_len),
            other => panic!("not truncated: {other:?}"),
        }
    }

    #[test]
    fn char_boundaries() {
        let s = "aé€"; // 1 + 2 + 3 bytes
        assert_eq!(floor_char_boundary(s, 0), 0);
        assert_eq!(floor_char_boundary(s, 2), 1);
        assert_eq!(floor_char_boundary(s, 3), 3);
        assert_eq!(floor_char_boundary(s, 5), 3);
        assert_eq!(floor_char_boundary(s, 6), 6);
        assert_eq!(floor_char_boundary(s, 100), 6);
        let raw = s.as_bytes();
        for max in 0..8 {
            assert_eq!(
                utf8_prefix_len(raw, max),
                floor_char_boundary(s, max),
                "{max}"
            );
        }
    }

    #[test]
    fn text_is_cut_on_a_character_boundary() {
        assert_eq!(truncate_text("abc".into(), 3), Value::Text("abc".into()));
        let (prefix, full) = cut(truncate_text("ééé".into(), 3));
        assert_eq!(prefix, Value::Text("é".into()));
        assert_eq!(full, Some(6));
        let (prefix, full) = cut(truncate_text("é".into(), 1));
        assert_eq!(prefix, Value::Text(String::new()));
        assert_eq!(full, Some(2));
        let (prefix, _) = cut(truncate_json(r#"{"key": 1}"#.into(), 6));
        assert_eq!(prefix, Value::Json(r#"{"key""#.into()));
        assert_eq!(truncate_json("[]".into(), 2), Value::Json("[]".into()));
    }

    #[test]
    fn raw_text_and_bytes() {
        let (prefix, full) = cut(text_from_utf8("ééé".as_bytes(), 3));
        assert_eq!(prefix, Value::Text("é".into()));
        assert_eq!(full, Some(6));
        assert_eq!(text_from_utf8(b"abc", 3), Value::Text("abc".into()));
        // Invalid bytes are replaced, the length is the raw length.
        assert_eq!(
            text_from_utf8(b"a\xffb", 10),
            Value::Text("a\u{fffd}b".into())
        );
        let (prefix, _) = cut(json_from_utf8(br#"{"key": "value"}"#, 6));
        assert_eq!(prefix, Value::Json(r#"{"key""#.into()));

        let (prefix, full) = cut(bytes_from_slice(&[1, 2, 3, 4], 2));
        assert_eq!(prefix, Value::Bytes(vec![1, 2]));
        assert_eq!(full, Some(4));
        assert_eq!(bytes_from_slice(&[1, 2], 2), Value::Bytes(vec![1, 2]));
        let (prefix, full) = cut(truncate_bytes(vec![1, 2, 3], 1));
        assert_eq!(prefix, Value::Bytes(vec![1]));
        assert_eq!(full, Some(3));
        assert_eq!(truncate_bytes(vec![], 0), Value::Bytes(vec![]));
    }

    #[test]
    fn normalizes_plain_decimals() {
        let n = |s: &str| normalize_decimal(s);
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
        let n = |s: &str| normalize_decimal(s);
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
        assert_eq!(n("1e-130").unwrap().len(), 132);
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
            assert_eq!(normalize_decimal(s), None, "{s:?}");
        }
    }

    #[test]
    fn pads_scale() {
        assert_eq!(pad_decimal_scale("5", 2), "5.00");
        assert_eq!(pad_decimal_scale("-0.5", 3), "-0.500");
        assert_eq!(pad_decimal_scale("1.2345", 2), "1.2345");
        assert_eq!(pad_decimal_scale("7", 0), "7");
    }
}
