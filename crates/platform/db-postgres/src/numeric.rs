//! Decoder for the binary `numeric` wire format, written from the public
//! PostgreSQL documentation (the "Numeric Types" chapter and the frontend/backend
//! protocol description).
//!
//! The wire form is a header of four 16-bit big-endian fields (`ndigits`,
//! `weight`, `sign`, `dscale`) followed by `ndigits` base-10000 digits. Digit
//! `i` is worth `10000^(weight - i)`. `dscale` is the number of decimal digits
//! after the decimal point in the text form.

const SIGN_POSITIVE: u16 = 0x0000;
const SIGN_NEGATIVE: u16 = 0x4000;
const SIGN_NAN: u16 = 0xC000;
const SIGN_POS_INF: u16 = 0xD000;
const SIGN_NEG_INF: u16 = 0xF000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Numeric {
    /// Plain decimal notation with exactly `dscale` fraction digits.
    Number(String),
    NaN,
    PosInfinity,
    NegInfinity,
}

impl Numeric {
    pub(crate) fn special_text(&self) -> Option<&'static str> {
        match self {
            Numeric::Number(_) => None,
            Numeric::NaN => Some("NaN"),
            Numeric::PosInfinity => Some("Infinity"),
            Numeric::NegInfinity => Some("-Infinity"),
        }
    }
}

/// Decode a binary `numeric`. `None` for malformed input.
pub(crate) fn decode(raw: &[u8]) -> Option<Numeric> {
    if raw.len() < 8 {
        return None;
    }
    let ndigits = i16::from_be_bytes([raw[0], raw[1]]);
    let weight = i16::from_be_bytes([raw[2], raw[3]]);
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let dscale = u16::from_be_bytes([raw[6], raw[7]]);
    match sign {
        SIGN_NAN => return Some(Numeric::NaN),
        SIGN_POS_INF => return Some(Numeric::PosInfinity),
        SIGN_NEG_INF => return Some(Numeric::NegInfinity),
        SIGN_POSITIVE | SIGN_NEGATIVE => {}
        _ => return None,
    }
    let nd = usize::try_from(ndigits).ok()?;
    if raw.len() != 8 + nd * 2 {
        return None;
    }
    let mut digits = Vec::with_capacity(nd);
    for chunk in raw[8..].as_chunks::<2>().0 {
        let d = u16::from_be_bytes(*chunk);
        if d > 9999 {
            return None;
        }
        digits.push(d);
    }
    Some(Numeric::Number(render(
        &digits,
        isize::from(weight),
        sign == SIGN_NEGATIVE,
        usize::from(dscale),
    )))
}

fn render(digits: &[u16], weight: isize, negative: bool, dscale: usize) -> String {
    let digit = |i: isize| -> u16 {
        usize::try_from(i)
            .ok()
            .and_then(|i| digits.get(i))
            .copied()
            .unwrap_or(0)
    };

    let mut int_part = String::new();
    if weight < 0 {
        int_part.push('0');
    } else {
        for i in 0..=weight {
            let d = digit(i);
            if i == 0 {
                int_part.push_str(&d.to_string());
            } else {
                int_part.push_str(&format!("{d:04}"));
            }
        }
        let trimmed = int_part.trim_start_matches('0');
        int_part = if trimmed.is_empty() {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        };
    }

    let mut out = String::with_capacity(int_part.len() + dscale + 2);
    if negative && !digits.is_empty() {
        out.push('-');
    }
    out.push_str(&int_part);
    if dscale > 0 {
        out.push('.');
        let mut frac = String::with_capacity(dscale + 4);
        for j in 1..=dscale.div_ceil(4) {
            frac.push_str(&format!("{:04}", digit(weight + j as isize)));
        }
        frac.truncate(dscale);
        out.push_str(&frac);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encode a decimal string into the wire format, for round-trip tests.
    pub(crate) fn encode(text: &str) -> Vec<u8> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (int_s, frac_s) = body.split_once('.').unwrap_or((body, ""));
        let dscale = frac_s.len();
        let int_pad = (4 - int_s.len() % 4) % 4;
        let frac_pad = (4 - frac_s.len() % 4) % 4;
        let padded = format!(
            "{}{}{}{}",
            "0".repeat(int_pad),
            int_s,
            frac_s,
            "0".repeat(frac_pad)
        );
        let int_groups = (int_s.len() + int_pad) / 4;
        let mut groups: Vec<u16> = padded
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap().parse().unwrap())
            .collect();
        let mut weight = int_groups as i16 - 1;
        while groups.first() == Some(&0) {
            groups.remove(0);
            weight -= 1;
        }
        while groups.last() == Some(&0) {
            groups.pop();
        }
        if groups.is_empty() {
            weight = 0;
        }
        let mut out = Vec::new();
        out.extend((groups.len() as i16).to_be_bytes());
        out.extend(weight.to_be_bytes());
        let sign = if negative && !groups.is_empty() {
            SIGN_NEGATIVE
        } else {
            SIGN_POSITIVE
        };
        out.extend(sign.to_be_bytes());
        out.extend((dscale as u16).to_be_bytes());
        for g in groups {
            out.extend(g.to_be_bytes());
        }
        out
    }

    fn header(ndigits: i16, weight: i16, sign: u16, dscale: u16, digits: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(ndigits.to_be_bytes());
        out.extend(weight.to_be_bytes());
        out.extend(sign.to_be_bytes());
        out.extend(dscale.to_be_bytes());
        for d in digits {
            out.extend(d.to_be_bytes());
        }
        out
    }

    fn num(text: &str) -> Numeric {
        Numeric::Number(text.to_owned())
    }

    #[test]
    fn zero_and_scaled_zero() {
        assert_eq!(decode(&header(0, 0, 0, 0, &[])), Some(num("0")));
        assert_eq!(decode(&header(0, 0, 0, 2, &[])), Some(num("0.00")));
        assert_eq!(decode(&header(0, 0, 0, 7, &[])), Some(num("0.0000000")));
    }

    #[test]
    fn small_and_fractional_values() {
        // 0.5, 0.0001, 0.00001 (weight -2), 1.50
        assert_eq!(decode(&header(1, -1, 0, 1, &[5000])), Some(num("0.5")));
        assert_eq!(decode(&header(1, -1, 0, 4, &[1])), Some(num("0.0001")));
        assert_eq!(decode(&header(1, -2, 0, 5, &[1000])), Some(num("0.00001")));
        assert_eq!(decode(&header(2, 0, 0, 2, &[1, 5000])), Some(num("1.50")));
    }

    #[test]
    fn integers_with_trailing_zero_groups() {
        // 10000 is stored as one digit with weight 1
        assert_eq!(decode(&header(1, 1, 0, 0, &[1])), Some(num("10000")));
        assert_eq!(
            decode(&header(1, 3, 0, 0, &[12])),
            Some(num("12000000000000"))
        );
        assert_eq!(decode(&header(1, 0, 0, 0, &[42])), Some(num("42")));
    }

    #[test]
    fn negative_values() {
        assert_eq!(
            decode(&header(2, 0, SIGN_NEGATIVE, 2, &[12, 3400])),
            Some(num("-12.34"))
        );
        assert_eq!(
            decode(&header(1, -1, SIGN_NEGATIVE, 3, &[10])),
            Some(num("-0.001"))
        );
        assert_eq!(
            decode(&header(1, 0, SIGN_NEGATIVE, 0, &[1])),
            Some(num("-1"))
        );
    }

    #[test]
    fn negative_zero_prints_without_sign() {
        assert_eq!(
            decode(&header(0, 0, SIGN_NEGATIVE, 2, &[])),
            Some(num("0.00"))
        );
    }

    #[test]
    fn scale_larger_than_stored_digits() {
        // 1.2 stored as {1, 2000} but displayed with scale 6
        assert_eq!(
            decode(&header(2, 0, 0, 6, &[1, 2000])),
            Some(num("1.200000"))
        );
        // scale not a multiple of four
        assert_eq!(decode(&header(2, 0, 0, 3, &[7, 1234])), Some(num("7.123")));
    }

    #[test]
    fn specials() {
        assert_eq!(decode(&header(0, 0, SIGN_NAN, 0, &[])), Some(Numeric::NaN));
        assert_eq!(
            decode(&header(0, 0, SIGN_POS_INF, 0, &[])),
            Some(Numeric::PosInfinity)
        );
        assert_eq!(
            decode(&header(0, 0, SIGN_NEG_INF, 0, &[])),
            Some(Numeric::NegInfinity)
        );
        assert_eq!(Numeric::NaN.special_text(), Some("NaN"));
        assert_eq!(Numeric::NegInfinity.special_text(), Some("-Infinity"));
        assert_eq!(num("1").special_text(), None);
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[0; 7]), None);
        // claims two digits, carries one
        assert_eq!(decode(&header(2, 0, 0, 0, &[1])), None);
        // digit out of range
        assert_eq!(decode(&header(1, 0, 0, 0, &[10000])), None);
        // unknown sign word
        assert_eq!(decode(&header(0, 0, 0x1234, 0, &[])), None);
        // negative digit count
        assert_eq!(decode(&header(-1, 0, 0, 0, &[])), None);
    }

    #[test]
    fn round_trip_many_values() {
        let cases = [
            "0",
            "1",
            "-1",
            "9",
            "10",
            "9999",
            "10000",
            "10001",
            "99999999",
            "100000000",
            "123456789",
            "-123456789",
            "0.1",
            "0.01",
            "0.001",
            "0.0001",
            "0.00001",
            "0.000000001",
            "-0.000000001",
            "1.0",
            "1.00",
            "12.3400",
            "100.5",
            "1000000.000001",
            "99999999999999999999999999999999999999",
            "-99999999999999999999999999999999999999",
            "12345678901234567890123456789012345678",
            "1234567890123456789012345678901234.5678",
            "0.12345678901234567890123456789012345678",
            "340282366920938463463374607431768211456",
            "1.000000000000000000000000000000000001",
            "70000000000000000000000000000000000000.000",
        ];
        for case in cases {
            let wire = encode(case);
            assert_eq!(decode(&wire), Some(num(case)), "case {case}");
        }
    }

    #[test]
    fn round_trip_generated_values() {
        // deterministic pseudo-random decimals of varied shape
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..2000 {
            let int_len = (next() % 40) as usize;
            let frac_len = (next() % 25) as usize;
            let mut s = String::new();
            if next() % 3 == 0 {
                s.push('-');
            }
            let mut int_digits: String = (0..int_len)
                .map(|_| char::from(b'0' + (next() % 10) as u8))
                .collect();
            int_digits = int_digits.trim_start_matches('0').to_owned();
            if int_digits.is_empty() {
                int_digits.push('0');
            }
            s.push_str(&int_digits);
            if frac_len > 0 {
                s.push('.');
                for _ in 0..frac_len {
                    s.push(char::from(b'0' + (next() % 10) as u8));
                }
            }
            // a negative number that is all zeros has no sign on the wire
            let all_zero = s.chars().all(|c| c == '0' || c == '.' || c == '-');
            let expected = if all_zero {
                s.trim_start_matches('-').to_owned()
            } else {
                s.clone()
            };
            assert_eq!(decode(&encode(&s)), Some(num(&expected)), "case {s}");
        }
    }
}
