//! Masking of identifying values for diagnostics that will be shared
//! (`--share`): host names, addresses, user, service and file paths.
//! Secrets are removed regardless of `--share`.

/// Replaces known sensitive strings with neutral placeholders.
#[derive(Debug, Default, Clone)]
pub struct Redactor {
    pairs: Vec<(String, String)>,
}

impl Redactor {
    /// Mask `value` as `placeholder` (case-insensitive). Values shorter than
    /// two characters are ignored so single letters do not get mangled.
    pub fn add(&mut self, value: &str, placeholder: &str) {
        let value = value.trim();
        if value.chars().count() < 2
            || self
                .pairs
                .iter()
                .any(|(v, _)| v.eq_ignore_ascii_case(value))
        {
            return;
        }
        self.pairs.push((value.to_owned(), placeholder.to_owned()));
        // Longest first so `db.example.com` wins over `example`.
        self.pairs.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
    }

    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (value, placeholder) in &self.pairs {
            out = replace_ignore_case(&out, value, placeholder);
        }
        out
    }
}

fn replace_ignore_case(haystack: &str, needle: &str, with: &str) -> String {
    let lower_h = haystack.to_lowercase();
    let lower_n = needle.to_lowercase();
    // Lowercasing can change byte lengths for some scripts; fall back to an
    // exact replace then.
    if lower_h.len() != haystack.len() || lower_n.len() != needle.len() {
        return haystack.replace(needle, with);
    }
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (i, _) in lower_h.match_indices(&lower_n) {
        if i < last {
            continue;
        }
        out.push_str(&haystack[last..i]);
        out.push_str(with);
        last = i + needle.len();
    }
    out.push_str(&haystack[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_case_insensitively_longest_first() {
        let mut r = Redactor::default();
        r.add("example", "<x>");
        r.add("db.example.com", "<host>");
        r.add("app_user", "<user>");
        let s = r.apply("ORA-01017 for APP_USER at db.example.com (example)");
        assert_eq!(s, "ORA-01017 for <user> at <host> (<x>)");
    }

    #[test]
    fn ignores_tiny_values() {
        let mut r = Redactor::default();
        r.add("a", "<x>");
        assert_eq!(r.apply("a cat"), "a cat");
    }
}
