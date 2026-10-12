//! Name matching of Pyoxide IDE, as the reference IDE matches names in Go
//! to File, Go to Action and speed search: a pattern such as `nut`,
//! `NaUT`, `*util*` or `na ut` finds `NameUtilTest` by its words (camel
//! humps, `_`, `-`, `.` and spaces), and a matching degree ranks the names
//! it finds.
//!
//! - Letters match word starts ("humps") in order; a lower-case letter may
//!   continue the current word or start the next one, an upper-case one
//!   starts a word. `*` skips anything; a space skips to a word start and
//!   at the end of the pattern asks for the last word.
//! - After a leading `*` a match may start in the middle of a word when it
//!   is at least three letters long.
//! - A pattern typed with the Russian keyboard layout on also matches as
//!   the same keys in the English layout (`ыек` finds `String`).
//!
//! Positions are char offsets into the name.
//!
//! No UI toolkit here: the crate is tested on its own.

// Matched fragments are lists of ranges, often of one.
#![allow(clippy::single_range_in_vec_init)]

mod layout;
mod minuscule;
mod words;

use std::ops::Range;

pub use layout::fix_layout;
pub use words::{is_word_start, next_word};

use minuscule::Minuscule;

/// How letter case counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchingMode {
    /// Case does not matter (it still counts in the degree).
    #[default]
    IgnoreCase,
    /// The first letter's case must match.
    FirstLetter,
    /// Every letter's case must match.
    MatchCase,
}

/// The degree a match at the start of the name gets on top with
/// [`Matcher::prefer_start`].
pub const START_MATCH_WEIGHT: i32 = 10_000;

/// A match: the matched fragments of the name and how good it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Char ranges of the name, in order.
    pub fragments: Vec<Range<usize>>,
    /// Higher is better.
    pub degree: i32,
}

impl Match {
    /// The match starts at the start of the name.
    pub fn is_start_match(&self) -> bool {
        self.fragments.first().is_none_or(|f| f.start == 0)
    }
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Matcher {
    pattern: String,
    main: Minuscule,
    /// The pattern on the English keys, when it was typed in another
    /// layout.
    fallback: Option<Minuscule>,
    prefer_start: bool,
}

impl Matcher {
    pub fn new(pattern: &str, mode: MatchingMode) -> Self {
        Self::with_separators(pattern, mode, "")
    }

    /// With hard separators: lower-case letters do not skip to words after
    /// them unless the pattern has a separator or a capital.
    pub fn with_separators(pattern: &str, mode: MatchingMode, separators: &str) -> Self {
        let fallback = layout::fix_layout(pattern)
            .filter(|fixed| fixed != pattern)
            .map(|fixed| Minuscule::new(&fixed, mode, separators));
        Self {
            pattern: pattern.to_owned(),
            main: Minuscule::new(pattern, mode, separators),
            fallback,
            prefer_start: false,
        }
    }

    /// A match at the start of the name gets [`START_MATCH_WEIGHT`] on top
    /// of its degree.
    pub fn prefer_start(mut self) -> Self {
        self.prefer_start = true;
        self
    }

    /// The pattern as given.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The pattern the matcher uses (a trailing `"* "` dropped).
    pub fn effective_pattern(&self) -> String {
        self.main.pattern().iter().collect()
    }

    pub fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.chars().collect();
        self.matches_chars(&name)
    }

    pub fn matches_chars(&self, name: &[char]) -> bool {
        self.main.find(name).is_some()
            || self
                .fallback
                .as_ref()
                .is_some_and(|f| f.find(name).is_some())
    }

    /// The fragments of `name` the pattern matches.
    pub fn find(&self, name: &str) -> Option<Vec<Range<usize>>> {
        let name: Vec<char> = name.chars().collect();
        self.find_chars(&name)
    }

    pub fn find_chars(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        self.matcher_for(name).1
    }

    /// The fragments and the degree (without the case of the first letter
    /// counting extra).
    pub fn match_chars(&self, name: &[char]) -> Option<Match> {
        self.match_with(name, false)
    }

    /// As [`Self::match_chars`]; with `value_start_case_match` a first
    /// letter in the same case as the pattern's counts extra.
    pub fn match_with(&self, name: &[char], value_start_case_match: bool) -> Option<Match> {
        let (matcher, fragments) = self.matcher_for(name);
        let fragments = fragments?;
        let mut degree = matcher.degree(name, value_start_case_match, Some(&fragments));
        if self.prefer_start && fragments.first().is_some_and(|f| f.start == 0) {
            degree += START_MATCH_WEIGHT;
        }
        Some(Match { fragments, degree })
    }

    /// The matching degree of `name`, `i32::MIN` when it does not match.
    pub fn degree(&self, name: &str) -> i32 {
        let name: Vec<char> = name.chars().collect();
        self.match_chars(&name).map_or(i32::MIN, |m| m.degree)
    }

    /// The main pattern's match, or the fixed layout's when the main one
    /// finds nothing.
    fn matcher_for(&self, name: &[char]) -> (&Minuscule, Option<Vec<Range<usize>>>) {
        let main = self.main.find(name);
        match &self.fallback {
            Some(fallback) if main.as_ref().is_none_or(|m| m.is_empty()) => {
                (fallback, fallback.find(name))
            }
            _ => (&self.main, main),
        }
    }
}

/// Every word of the pattern starts a word of the name, in any order
/// (words split at spaces, `-` and `/`): "find usages" finds "Find
/// Usages in File". The reference matches action descriptions, and
/// patterns with spaces, this way.
#[derive(Debug, Clone)]
pub struct WordPrefixMatcher {
    words: Vec<String>,
    fallback: Option<Vec<String>>,
}

impl WordPrefixMatcher {
    pub fn new(pattern: &str) -> Self {
        let fallback = fix_layout(pattern)
            .filter(|fixed| fixed != pattern)
            .map(|fixed| prefix_words(&fixed));
        Self {
            words: prefix_words(pattern),
            fallback,
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        let names = prefix_words(name);
        let all = |words: &[String]| {
            words
                .iter()
                .all(|w| names.iter().any(|n| starts_with_ignore_case(n, w)))
        };
        all(&self.words) || self.fallback.as_deref().is_some_and(all)
    }
}

fn prefix_words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '-' || c == '/')
        .map(str::to_owned)
        .collect()
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    let mut t = text.chars();
    prefix
        .chars()
        .all(|p| t.next().is_some_and(|c| words::eq_ignore_case(c, p)))
}

#[cfg(test)]
mod tests;
