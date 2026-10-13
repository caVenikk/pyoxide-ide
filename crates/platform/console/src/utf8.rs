/// Incremental UTF-8 decoding: a character split between chunks is joined,
/// invalid bytes become U+FFFD (as the reference's stream decoder replaces
/// malformed input).
#[derive(Debug, Default)]
pub(crate) struct Utf8Decoder {
    carry: [u8; 4],
    len: usize,
}

impl Utf8Decoder {
    /// Decodes `bytes`, appending the text to `out`.
    pub(crate) fn decode(&mut self, mut bytes: &[u8], out: &mut String) {
        if self.len > 0 {
            let take = bytes.len().min(4 - self.len);
            let mut tmp = [0u8; 4];
            tmp[..self.len].copy_from_slice(&self.carry[..self.len]);
            tmp[self.len..self.len + take].copy_from_slice(&bytes[..take]);
            let total = self.len + take;
            match std::str::from_utf8(&tmp[..total]) {
                Ok(s) => {
                    out.push_str(s);
                    bytes = &bytes[take..];
                }
                Err(e) if e.valid_up_to() > 0 => {
                    let valid = e.valid_up_to();
                    out.push_str(std::str::from_utf8(&tmp[..valid]).unwrap_or_default());
                    bytes = &bytes[valid.saturating_sub(self.len)..];
                }
                Err(e) => match e.error_len() {
                    None => {
                        // Still incomplete: everything taken is carried.
                        self.carry[..total].copy_from_slice(&tmp[..total]);
                        self.len = total;
                        return;
                    }
                    Some(n) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        bytes = &bytes[n.saturating_sub(self.len)..];
                    }
                },
            }
            self.len = 0;
        }
        loop {
            match std::str::from_utf8(bytes) {
                Ok(s) => {
                    out.push_str(s);
                    return;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.push_str(std::str::from_utf8(&bytes[..valid]).unwrap_or_default());
                    match e.error_len() {
                        None => {
                            let rest = &bytes[valid..];
                            self.carry[..rest.len()].copy_from_slice(rest);
                            self.len = rest.len();
                            return;
                        }
                        Some(n) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            bytes = &bytes[valid + n..];
                        }
                    }
                }
            }
        }
    }

    /// The input ended: an incomplete character becomes U+FFFD.
    pub(crate) fn finish(&mut self, out: &mut String) {
        if self.len > 0 {
            out.push(char::REPLACEMENT_CHARACTER);
            self.len = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]]) -> String {
        let mut d = Utf8Decoder::default();
        let mut out = String::new();
        for c in chunks {
            d.decode(c, &mut out);
        }
        d.finish(&mut out);
        out
    }

    #[test]
    fn characters_split_anywhere_are_joined() {
        let text = "aé€😀b";
        let bytes = text.as_bytes();
        for i in 0..=bytes.len() {
            for j in i..=bytes.len() {
                assert_eq!(
                    decode(&[&bytes[..i], &bytes[i..j], &bytes[j..]]),
                    text,
                    "{i} {j}"
                );
            }
        }
        // One byte at a time.
        let chunks: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(decode(&chunks), text);
    }

    #[test]
    fn invalid_bytes_become_replacement_characters() {
        assert_eq!(decode(&[b"a\xffb"]), "a\u{FFFD}b");
        assert_eq!(decode(&[b"a\xe2\x82", b"b"]), "a\u{FFFD}b");
        assert_eq!(decode(&[b"a\xe2", b"\x82"]), "a\u{FFFD}");
        assert_eq!(decode(&[b"\xf0\x9f", b"\x98", b"x"]), "\u{FFFD}x");
    }
}
