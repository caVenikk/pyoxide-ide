//! Queries, their compiled form and finding matches.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::cancel::{CancelToken, Cancelled};

/// How a query matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SearchOptions {
    /// Letters must match in case ("Match Case").
    pub case_sensitive: bool,
    /// A match must be a whole word ("Words"), see [`Matcher::find_at`].
    pub whole_words: bool,
    /// The text is a regular expression ("Regex").
    pub regex: bool,
}

/// What to search for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    pub options: SearchOptions,
}

/// Why a query cannot be searched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// The text is empty: nothing to search for.
    Empty,
    /// A regular expression made only of `|`: it matches nothing but the
    /// empty string everywhere.
    Vague,
    /// The regular expression does not compile; the parser's message.
    InvalidRegex(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Empty => f.write_str("nothing to search for"),
            QueryError::Vague => f.write_str("the pattern matches only empty strings"),
            QueryError::InvalidRegex(message) => {
                write!(f, "invalid regular expression: {message}")
            }
        }
    }
}

impl std::error::Error for QueryError {}

impl Query {
    pub fn new(text: impl Into<String>, options: SearchOptions) -> Self {
        Self {
            text: text.into(),
            options,
        }
    }

    /// Compiles the query. Plain text is matched literally; a regular
    /// expression uses the `regex` crate's syntax with `^` and `$` matching
    /// at line breaks. Without "Match Case" letters match in any case
    /// (Unicode simple case folding).
    pub fn compile(&self) -> Result<Matcher, QueryError> {
        if self.text.is_empty() {
            return Err(QueryError::Empty);
        }
        let regex_mode = self.options.regex;
        if regex_mode && self.text.chars().all(|c| c == '|') {
            return Err(QueryError::Vague);
        }
        let pattern = if regex_mode {
            self.text.clone()
        } else {
            // Text over several lines: a line break matches `\n` and
            // `\r\n`, so raw file text with CRLF breaks matches too.
            self.text
                .split('\n')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join(r"\r?\n")
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!self.options.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .map_err(|e| QueryError::InvalidRegex(e.to_string()))?;
        Ok(Matcher {
            regex,
            whole_words: self.options.whole_words,
            regex_mode,
        })
    }
}

/// A compiled query. Cheap to clone (the compiled program is shared) and
/// usable from any thread.
#[derive(Debug, Clone)]
pub struct Matcher {
    pub(crate) regex: Regex,
    whole_words: bool,
    pub(crate) regex_mode: bool,
}

/// What [`Matcher::find_all`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Byte ranges of the matches, in order, not overlapping.
    pub ranges: Vec<Range<usize>>,
    /// The search stopped at the limit; there are more matches.
    pub truncated: bool,
}

/// How many matches pass between two looks at the cancel token.
const CANCEL_CHECK_INTERVAL: usize = 256;

impl Matcher {
    /// Whether the query is a regular expression (its replacement is a
    /// template).
    pub fn is_regex(&self) -> bool {
        self.regex_mode
    }

    /// The first match that starts at or after byte `from` (a char
    /// boundary).
    ///
    /// With "Words" a match must not be part of a larger word: at each end
    /// a word character ([`is_word_char`]) must not have a word character
    /// next to it outside the match, and another character must not be
    /// repeated there (`==` is not found in `===`). A word character right
    /// after a backslash does not count as a neighbour, so `n` in `\name`
    /// does not hide the word `name`. A match that fails this is skipped
    /// and the search goes on one character after its start.
    pub fn find_at(&self, haystack: &str, from: usize) -> Option<Range<usize>> {
        self.find_from(haystack, from, None).unwrap_or(None)
    }

    /// [`Matcher::find_at`] that also looks at `cancel` while it skips
    /// matches that are not whole words.
    fn find_from(
        &self,
        haystack: &str,
        from: usize,
        cancel: Option<&CancelToken>,
    ) -> Result<Option<Range<usize>>, Cancelled> {
        let mut from = from;
        let mut skipped = 0usize;
        while from <= haystack.len() {
            let Some(m) = self.regex.find_at(haystack, from) else {
                return Ok(None);
            };
            if !self.whole_words || is_whole_word(haystack, m.start(), m.end()) {
                return Ok(Some(m.range()));
            }
            skipped += 1;
            if skipped.is_multiple_of(CANCEL_CHECK_INTERVAL)
                && cancel.is_some_and(|c| c.is_cancelled())
            {
                return Err(Cancelled);
            }
            from = next_boundary(haystack, m.start());
        }
        Ok(None)
    }

    /// Matches in the whole text, in order.
    pub fn find_iter<'h>(&'h self, haystack: &'h str) -> Matches<'h> {
        self.find_iter_in(haystack, 0..haystack.len())
    }

    /// Matches that lie inside `scope` (byte range of char boundaries), in
    /// order. Text outside the scope still counts for `^`, `$`, word
    /// boundaries and "Words". After a match the search continues at its
    /// end, after an empty match one character further; no search starts at
    /// the end of the scope, so an empty match there is reported only when
    /// a search that started before it finds it (`a*` finds two matches in
    /// `ab`, `^` finds three in `a\nb\n`).
    pub fn find_iter_in<'h>(&'h self, haystack: &'h str, scope: Range<usize>) -> Matches<'h> {
        let end = scope.end.min(haystack.len());
        Matches {
            matcher: self,
            haystack,
            pos: scope.start.min(end),
            end,
            cancel: None,
            cancelled: false,
        }
    }

    /// Every match inside `scope`, at most `limit` of them. Stops with
    /// [`Cancelled`] once `cancel` is set (checked between matches).
    pub fn find_all(
        &self,
        haystack: &str,
        scope: Range<usize>,
        limit: usize,
        cancel: &CancelToken,
    ) -> Result<Found, Cancelled> {
        if cancel.is_cancelled() {
            return Err(Cancelled);
        }
        let mut found = Found::default();
        let mut matches = self.find_iter_in(haystack, scope);
        matches.cancel = Some(cancel);
        for (i, m) in (&mut matches).enumerate() {
            if i % CANCEL_CHECK_INTERVAL == CANCEL_CHECK_INTERVAL - 1 && cancel.is_cancelled() {
                return Err(Cancelled);
            }
            if found.ranges.len() == limit {
                found.truncated = true;
                break;
            }
            found.ranges.push(m);
        }
        if matches.cancelled || cancel.is_cancelled() {
            return Err(Cancelled);
        }
        Ok(found)
    }
}

/// Iterator over the matches of a [`Matcher`] (byte ranges).
#[derive(Debug, Clone)]
pub struct Matches<'h> {
    matcher: &'h Matcher,
    haystack: &'h str,
    pos: usize,
    end: usize,
    cancel: Option<&'h CancelToken>,
    /// The search stopped because `cancel` was set.
    cancelled: bool,
}

impl Iterator for Matches<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        if self.pos >= self.end {
            return None;
        }
        let m = match self.matcher.find_from(self.haystack, self.pos, self.cancel) {
            Ok(m) => m,
            Err(Cancelled) => {
                self.cancelled = true;
                None
            }
        };
        match m {
            Some(m) if m.end <= self.end => {
                self.pos = if m.is_empty() {
                    next_boundary(self.haystack, m.end)
                } else {
                    m.end
                };
                Some(m)
            }
            _ => {
                self.pos = self.end;
                None
            }
        }
    }
}

/// `text` as a regular expression that matches it literally (selected text
/// put into a search field while "Regex" is on); line breaks become `\n`,
/// so the pattern stays on one line, as in the reference IDE.
pub fn escape(text: &str) -> String {
    regex::escape(text).replace('\n', "\\n")
}

/// A character that belongs to a word for "Words": a letter or digit of
/// any script, or `_`.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The char boundary after `pos` (one past the end at the end).
fn next_boundary(text: &str, pos: usize) -> usize {
    pos + text[pos..].chars().next().map_or(1, char::len_utf8)
}

/// See [`Matcher::find_at`].
fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let mut before = text[..start].chars().rev();
    let prev = before.next();
    let prev_escaped = before.next() == Some('\\');
    let first = text[start..].chars().next();
    let starts_word = match (prev, first) {
        (Some(p), Some(f)) if is_word_char(f) => !is_word_char(p) || prev_escaped,
        (Some(p), Some(f)) => p != f,
        _ => true,
    };
    let last = text[..end].chars().next_back();
    let next = text[end..].chars().next();
    let ends_word = match (last, next) {
        (Some(l), Some(n)) if is_word_char(l) => !is_word_char(n),
        (Some(l), Some(n)) => l != n,
        _ => true,
    };
    starts_word && ends_word
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // expected matches are lists of ranges
mod tests {
    use super::*;

    fn opts(case_sensitive: bool, whole_words: bool, regex: bool) -> SearchOptions {
        SearchOptions {
            case_sensitive,
            whole_words,
            regex,
        }
    }

    /// The matched texts.
    fn find(text: &str, query: &str, options: SearchOptions) -> Vec<&'static str> {
        let matcher = Query::new(query, options).compile().unwrap();
        matcher
            .find_iter(text)
            .map(|r| &*text[r].to_owned().leak())
            .collect()
    }

    fn ranges(text: &str, query: &str, options: SearchOptions) -> Vec<Range<usize>> {
        Query::new(query, options)
            .compile()
            .unwrap()
            .find_iter(text)
            .collect()
    }

    #[test]
    fn plain_text_ignores_case_unless_asked() {
        let text = "Foo foo FOO fOo";
        assert_eq!(find(text, "foo", opts(false, false, false)).len(), 4);
        assert_eq!(find(text, "foo", opts(true, false, false)), ["foo"]);
        assert_eq!(find(text, "FOO", opts(true, false, false)), ["FOO"]);
        // Non-ASCII letters fold too.
        assert_eq!(
            find("Ärger ärger", "ÄRGER", opts(false, false, false)).len(),
            2
        );
        assert_eq!(
            find("Ärger ärger", "ärger", opts(true, false, false)),
            ["ärger"]
        );
    }

    #[test]
    fn plain_text_is_literal() {
        let text = "a.b axb (x) [y] $1 \\d";
        assert_eq!(find(text, "a.b", opts(false, false, false)), ["a.b"]);
        assert_eq!(find(text, "(x)", opts(false, false, false)), ["(x)"]);
        assert_eq!(find(text, "$1", opts(false, false, false)), ["$1"]);
        assert_eq!(find(text, "\\d", opts(false, false, false)), ["\\d"]);
        // Matches do not overlap.
        assert_eq!(
            ranges("aaaa", "aa", opts(false, false, false)),
            [0..2, 2..4]
        );
        // A query across a line break.
        assert_eq!(
            ranges("x\ny\nx\ny", "x\ny", opts(false, false, false)),
            [0..3, 4..7]
        );
    }

    #[test]
    fn whole_words_use_unicode_letters_digits_and_underscore() {
        let w = opts(false, true, false);
        assert_eq!(
            ranges("foo food _foo foo_ foo1 foo", "foo", w),
            [0..3, 24..27]
        );
        // Letters of other scripts are word characters.
        assert_eq!(ranges("слово словарь слово", "слово", w).len(), 2);
        assert!(ranges("éfoo fooé", "foo", w).is_empty());
        // Punctuation around a word is a boundary.
        assert_eq!(find("(foo) foo.bar foo-x", "foo", w), ["foo", "foo", "foo"]);
        // A word after a backslash escape still starts a word.
        assert_eq!(find("\\nfoo nfoo", "foo", w), ["foo"]);
        // An operator is not found inside a longer run of itself.
        assert!(ranges("a === b", "==", w).is_empty());
        assert_eq!(ranges("a == b", "==", w), [2..4]);
        // A rejected candidate does not hide one that starts inside it.
        assert_eq!(ranges("aaa aa", "aa", w), [4..6]);
        // Words apply to regular expressions too.
        assert_eq!(
            find("cat cats concat cat", r"c\w+", opts(false, true, true)),
            ["cat", "cats", "concat", "cat"]
        );
        assert_eq!(
            find("cat cats concat", r"cat", opts(false, true, true)),
            ["cat"]
        );
    }

    #[test]
    fn regular_expressions_match_lines_and_groups() {
        let r = opts(true, false, true);
        let text = "def f(a):\n    return a\n\ndef g(b):\n    pass\n";
        assert_eq!(find(text, r"def (\w+)", r), ["def f", "def g"]);
        // ^ and $ match at every line.
        assert_eq!(
            find(text, r"^\s+\w+", r),
            ["    return", "\ndef", "    pass"]
        );
        assert_eq!(
            ranges("ab\ncd", "$", r),
            [2..2, 5..5],
            "$ before each line break and at the end"
        );
        assert_eq!(ranges("ab\ncd", "^", r), [0..0, 3..3]);
        // Case folding applies to classes and literals.
        assert_eq!(find("Abc aBC", "abc", opts(false, false, true)).len(), 2);
        assert_eq!(find("Abc aBC", "abc", r).len(), 0);
    }

    #[test]
    fn zero_length_matches_advance_by_one_character() {
        let r = opts(true, false, true);
        // Empty matches between characters, the nonempty run, and no
        // search starting at the very end.
        assert_eq!(ranges("baaab", "a*", r), [0..0, 1..4, 4..4]);
        // Multibyte characters are stepped over whole.
        assert_eq!(ranges("äö", "x*", r), [0..0, 2..2]);
        // Empty text: nothing.
        assert!(ranges("", "^", r).is_empty());
        assert_eq!(ranges("\n\n", "^$", r), [0..0, 1..1]);
    }

    #[test]
    fn crlf_line_breaks_count_as_one() {
        let r = opts(true, false, true);
        let text = "one\r\ntwo\r\n";
        assert_eq!(find(text, r"\w+$", r), ["one", "two"]);
        // The empty last line after the final break has a start too.
        assert_eq!(ranges(text, "^", r), [0..0, 5..5, 10..10]);
        // `.` does not match the carriage return.
        assert_eq!(find(text, r"two.", r), Vec::<&str>::new());
        assert_eq!(find(text, r"t.o", r), ["two"]);
    }

    #[test]
    fn queries_over_several_lines() {
        let text = "def f():\n    return 1\n\ndef g():\n    return 2\n";
        // Plain text across a line break.
        let plain = opts(true, false, false);
        assert_eq!(find(text, ":\n    return", plain).len(), 2);
        // A text with CRLF breaks matches the same query.
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(find(&crlf, ":\n    return", plain), [":\r\n    return"; 2]);
        // An escaped selection over lines finds itself as a regex.
        let selected = "f():\n    return 1";
        let pattern = escape(selected);
        assert_eq!(pattern, "f\\(\\):\\n    return 1");
        assert_eq!(find(text, &pattern, opts(true, false, true)), [selected]);
        // `\n` in a pattern.
        assert_eq!(find(text, r"\d\n\ndef", opts(true, false, true)).len(), 1);
        // Words: only the outer ends count.
        let words = opts(true, true, false);
        assert_eq!(find("ab\ncd abc\ncd ab\ncde", "ab\ncd", words), ["ab\ncd"]);
    }

    #[test]
    fn errors_and_vague_patterns() {
        let r = opts(false, false, true);
        assert_eq!(Query::new("", r).compile().unwrap_err(), QueryError::Empty);
        assert_eq!(
            Query::new("||", r).compile().unwrap_err(),
            QueryError::Vague
        );
        assert!(matches!(
            Query::new("(unclosed", r).compile().unwrap_err(),
            QueryError::InvalidRegex(_)
        ));
        assert!(matches!(
            Query::new("a{2,1}", r).compile().unwrap_err(),
            QueryError::InvalidRegex(_)
        ));
        // The same text is fine as plain text.
        let plain = Query::new("(unclosed", opts(false, false, false));
        assert_eq!(plain.compile().unwrap().find_iter("x (unclosed").count(), 1);
        assert_eq!(
            Query::new("|", opts(false, false, false))
                .compile()
                .unwrap()
                .find_iter("a|b")
                .count(),
            1
        );
    }

    #[test]
    fn scopes_limits_and_cancellation() {
        let m = Query::new("ab", opts(false, false, false))
            .compile()
            .unwrap();
        let text = "ab ab ab ab";
        // Only matches inside the scope.
        assert_eq!(m.find_iter_in(text, 2..7).collect::<Vec<_>>(), [3..5]);
        assert_eq!(m.find_iter_in(text, 3..8).collect::<Vec<_>>(), [3..5, 6..8]);
        let token = CancelToken::new();
        let all = m.find_all(text, 0..text.len(), 2, &token).unwrap();
        assert_eq!(all.ranges, [0..2, 3..5]);
        assert!(all.truncated);
        let all = m.find_all(text, 0..text.len(), 10, &token).unwrap();
        assert_eq!(all.ranges.len(), 4);
        assert!(!all.truncated);
        token.cancel();
        assert_eq!(m.find_all(text, 0..text.len(), 10, &token), Err(Cancelled));
        // A clone shares the flag.
        let other = CancelToken::new();
        other.clone().cancel();
        assert!(other.is_cancelled());
    }

    #[test]
    fn regex_scope_sees_context_outside() {
        let m = Query::new(r"^\w", opts(true, false, true))
            .compile()
            .unwrap();
        let text = "ab\ncd";
        // `b` is not at a line start even though the scope starts there.
        assert_eq!(m.find_iter_in(text, 1..5).collect::<Vec<_>>(), [3..4]);
    }
}
