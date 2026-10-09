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

/// Word characters for match boundaries: a value inside a longer
/// identifier (`pg` in `PXTEST_PG_PW`) is not masked.
fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn replace_ignore_case(haystack: &str, needle: &str, with: &str) -> String {
    let lower_h = haystack.to_lowercase();
    let lower_n = needle.to_lowercase();
    // Lowercasing can change byte lengths for some scripts; fall back to an
    // exact match then.
    let (h, n) = if lower_h.len() == haystack.len() && lower_n.len() == needle.len() {
        (lower_h.as_str(), lower_n.as_str())
    } else {
        (haystack, needle)
    };
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (i, _) in h.match_indices(n) {
        if i < last {
            continue;
        }
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + needle.len()..].chars().next();
        // Only whole words; values that start or end with punctuation
        // (paths, addresses) match anywhere.
        let starts_word = needle
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let ends_word = needle
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if (starts_word && is_word(before)) || (ends_word && is_word(after)) {
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
    fn masks_whole_words_only() {
        let mut r = Redactor::default();
        r.add("pg", "<profile>");
        r.add("pyoxide", "<user>");
        let s = r.apply("profile pg uses PXTEST_PG_PW and PYOXIDE_DB_PASSWORD as pyoxide");
        assert_eq!(
            s,
            "profile <profile> uses PXTEST_PG_PW and PYOXIDE_DB_PASSWORD as <user>"
        );
        r.add("10.0.0.5", "<ip1>");
        assert_eq!(r.apply("[10.0.0.5]:1521"), "[<ip1>]:1521");
    }

    #[test]
    fn ignores_tiny_values() {
        let mut r = Redactor::default();
        r.add("a", "<x>");
        assert_eq!(r.apply("a cat"), "a cat");
    }
}
