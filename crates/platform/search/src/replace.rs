//! Replacement text: the template as is for plain text; for a regular
//! expression, the template with group references, escapes and case
//! conversion expanded.
//!
//! Template syntax (the reference IDE's, see NOTICE):
//! - `$n`: group `n`; further digits are taken while the number names a
//!   group of the expression (`$10` is group 10 when there are ten groups,
//!   else group 1 and `0`); `${name}`: a named group. A group that did not
//!   take part in the match gives nothing.
//! - `\n`, `\r`, `\t`, `\f`, `\b` (backspace): control characters;
//!   `\xhhhh`: the character with that hexadecimal code.
//! - `\u`, `\l`: the next character in upper or lower case; `\U`, `\L`:
//!   everything up to `\E` (or the end) in upper or lower case.
//! - `\` before any other character: that character (`\$`, `\\`).

use std::ops::Range;

use regex::Captures;

use crate::matcher::Matcher;

/// Why a replacement template cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceError {
    /// The template is malformed; what is wrong.
    Malformed(String),
    /// The text at the match changed since it was found.
    Stale,
}

impl std::fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplaceError::Malformed(message) => {
                write!(f, "malformed replacement string: {message}")
            }
            ReplaceError::Stale => f.write_str("the text changed since the search"),
        }
    }
}

impl std::error::Error for ReplaceError {}

fn malformed(message: impl Into<String>) -> ReplaceError {
    ReplaceError::Malformed(message.into())
}

impl Matcher {
    /// The text that replaces the match `range` (byte range found by this
    /// matcher in `haystack`) for `template`.
    pub fn replacement(
        &self,
        haystack: &str,
        range: Range<usize>,
        template: &str,
    ) -> Result<String, ReplaceError> {
        if !self.regex_mode {
            return Ok(template.to_owned());
        }
        let captures = self
            .regex
            .captures_at(haystack, range.start)
            .filter(|c| c.get(0).map(|m| m.range()) == Some(range.clone()))
            .ok_or(ReplaceError::Stale)?;
        expand(template, &captures, self)
    }

    /// Checks a template against this query's groups without a match.
    pub fn check_template(&self, template: &str) -> Result<(), ReplaceError> {
        if !self.regex_mode {
            return Ok(());
        }
        Expander {
            matcher: self,
            captures: None,
        }
        .run(template)
        .map(drop)
    }
}

fn expand(
    template: &str,
    captures: &Captures<'_>,
    matcher: &Matcher,
) -> Result<String, ReplaceError> {
    Expander {
        matcher,
        captures: Some(captures),
    }
    .run(template)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Upper,
    Lower,
}

/// Output with the pending case conversion.
#[derive(Default)]
struct Output {
    text: String,
    /// Case of everything up to `\E`.
    region: Option<Case>,
    /// Case of the next character only (ignored inside a region).
    next: Option<Case>,
}

impl Output {
    fn push(&mut self, c: char) {
        match self.next.take().or(self.region) {
            Some(Case::Upper) => self.text.extend(c.to_uppercase()),
            Some(Case::Lower) => self.text.extend(c.to_lowercase()),
            None => self.text.push(c),
        }
    }

    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            self.push(c);
        }
    }

    fn next_char(&mut self, case: Case) {
        if self.region.is_none() {
            self.next = Some(case);
        }
    }

    fn start_region(&mut self, case: Case) {
        self.next = None;
        self.region = Some(case);
    }

    fn end_region(&mut self) {
        self.next = None;
        self.region = None;
    }
}

struct Expander<'a, 'h> {
    matcher: &'a Matcher,
    /// `None` when only checking the template.
    captures: Option<&'a Captures<'h>>,
}

impl Expander<'_, '_> {
    fn run(&self, template: &str) -> Result<String, ReplaceError> {
        let mut out = Output::default();
        let mut chars = template.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            match c {
                '\\' => {
                    let Some((at, e)) = chars.next() else {
                        return Err(malformed("character to be escaped is missing"));
                    };
                    match e {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'f' => out.push('\u{c}'),
                        'b' => out.push('\u{8}'),
                        'x' => {
                            let start = at + 1;
                            let code = template
                                .get(start..start + 4)
                                .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .and_then(char::from_u32);
                            if let Some(code) = code {
                                for _ in 0..4 {
                                    chars.next();
                                }
                                out.push(code);
                            }
                        }
                        'u' => out.next_char(Case::Upper),
                        'l' => out.next_char(Case::Lower),
                        'U' => out.start_region(Case::Upper),
                        'L' => out.start_region(Case::Lower),
                        'E' => out.end_region(),
                        other => out.push(other),
                    }
                }
                '$' => match chars.next() {
                    None => return Err(malformed("group index is missing")),
                    Some((_, '{')) => {
                        let mut name = String::new();
                        while let Some(&(_, n)) = chars.peek() {
                            if n.is_ascii_alphanumeric() {
                                name.push(n);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        if name.is_empty() {
                            return Err(malformed("named capturing group has 0 length name"));
                        }
                        if chars.next().map(|(_, c)| c) != Some('}') {
                            return Err(malformed("named capturing group is missing trailing '}'"));
                        }
                        if name.starts_with(|c: char| c.is_ascii_digit()) {
                            return Err(malformed(format!(
                                "capturing group name {{{name}}} starts with digit character"
                            )));
                        }
                        if !self
                            .matcher
                            .regex
                            .capture_names()
                            .any(|n| n == Some(name.as_str()))
                        {
                            return Err(malformed(format!("no group with name {{{name}}}")));
                        }
                        if let Some(m) = self.captures.and_then(|c| c.name(&name)) {
                            out.push_str(m.as_str());
                        }
                    }
                    Some((_, d)) if d.is_ascii_digit() => {
                        let groups = self.matcher.regex.captures_len() - 1;
                        let mut number = d as usize - '0' as usize;
                        while let Some(&(_, n)) = chars.peek() {
                            let Some(digit) = n.to_digit(10) else { break };
                            let longer = number * 10 + digit as usize;
                            if longer > groups {
                                break;
                            }
                            number = longer;
                            chars.next();
                        }
                        if number > groups {
                            return Err(malformed(format!("no group {number}")));
                        }
                        if let Some(m) = self.captures.and_then(|c| c.get(number)) {
                            out.push_str(m.as_str());
                        }
                    }
                    Some(_) => return Err(malformed("illegal group reference")),
                },
                c => out.push(c),
            }
        }
        Ok(out.text)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Query, SearchOptions};

    use super::*;

    fn regex(pattern: &str) -> Matcher {
        Query::new(
            pattern,
            SearchOptions {
                case_sensitive: true,
                regex: true,
                ..SearchOptions::default()
            },
        )
        .compile()
        .unwrap()
    }

    /// Replaces every match of `pattern` in `text` with `template`.
    fn replace_all(text: &str, pattern: &str, template: &str) -> Result<String, ReplaceError> {
        let m = regex(pattern);
        let mut out = String::new();
        let mut last = 0;
        for r in m.find_iter(text) {
            out.push_str(&text[last..r.start]);
            out.push_str(&m.replacement(text, r.clone(), template)?);
            last = r.end;
        }
        out.push_str(&text[last..]);
        Ok(out)
    }

    #[test]
    fn plain_text_replacement_is_literal() {
        let m = Query::new("a", SearchOptions::default()).compile().unwrap();
        assert_eq!(m.replacement("a", 0..1, r"$1\n").unwrap(), r"$1\n");
        assert!(m.check_template("$").is_ok());
    }

    #[test]
    fn groups_by_number_and_name() {
        assert_eq!(
            replace_all("key=value, a=b", r"(\w+)=(\w+)", "$2=$1").unwrap(),
            "value=key, b=a"
        );
        assert_eq!(
            replace_all("f(x)", r"(?<name>\w+)\((?P<arg>\w+)\)", "${arg}.${name}()").unwrap(),
            "x.f()"
        );
        // $0 is the whole match; a group that did not match gives nothing.
        assert_eq!(replace_all("ab", r"a(x)?b", "[$0|$1]").unwrap(), "[ab|]");
        // Digits after a group number belong to it only while such a group exists.
        assert_eq!(replace_all("ab", r"(a)(b)", "$10").unwrap(), "a0");
        let ten = r"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)";
        assert_eq!(replace_all("abcdefghij", ten, "$10$1").unwrap(), "ja");
    }

    #[test]
    fn escapes_and_case_conversion() {
        assert_eq!(replace_all("a b", " ", r"\n\t\\\$").unwrap(), "a\n\t\\$b");
        assert_eq!(replace_all("x", "x", r"\x0041\x00e9").unwrap(), "Aé");
        // Not four hex digits: the escape gives nothing.
        assert_eq!(replace_all("x", "x", r"\xZZ").unwrap(), "ZZ");
        assert_eq!(
            replace_all("hello world", r"(\w+) (\w+)", r"\u$1 \U$2\E!").unwrap(),
            "Hello WORLD!"
        );
        assert_eq!(
            replace_all("MAKE_LOWER", r"(\w+)", r"\L$1").unwrap(),
            "make_lower"
        );
        assert_eq!(replace_all("Name", r"(\w+)", r"\l$1").unwrap(), "name");
    }

    #[test]
    fn zero_length_matches_are_replaced_everywhere() {
        assert_eq!(replace_all("a\nb", "^", "# ").unwrap(), "# a\n# b");
        // The empty line after the last break starts a line too.
        assert_eq!(replace_all("a\n", "^", "# ").unwrap(), "# a\n# ");
        assert_eq!(replace_all("a\nb", "$", ";").unwrap(), "a;\nb;");
    }

    #[test]
    fn malformed_templates() {
        let m = regex(r"(a)");
        for bad in [
            r"\", "$", "$x", "$2", "${", "${}", "${a", "${1a}", "${nope}",
        ] {
            assert!(
                matches!(m.check_template(bad), Err(ReplaceError::Malformed(_))),
                "{bad:?}"
            );
            assert!(
                matches!(
                    m.replacement("a", 0..1, bad),
                    Err(ReplaceError::Malformed(_))
                ),
                "{bad:?}"
            );
        }
        assert!(m.check_template(r"$1 \$ \\").is_ok());
        // A match that is no longer there.
        assert_eq!(m.replacement("b", 0..1, "x"), Err(ReplaceError::Stale));
    }
}
