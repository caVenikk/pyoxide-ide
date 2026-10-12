//! The camel-hump matcher itself: the pattern's letters are found in the
//! name in order, each run of them at a word start (or, after `*`, anywhere
//! when at least three letters long); `*` and space skip anything, a space
//! also stands for a word separator. Upper-case pattern letters must match
//! word starts; lower-case ones may also match them ("nu" finds
//! `NameUtil`).

use std::ops::Range;

use crate::MatchingMode;
use crate::words::{
    eq_ignore_case, is_digit, is_letter_or_digit, is_lower, is_upper, is_word_start, lower,
    next_word, upper,
};

/// Patterns longer than this match as a prefix only (camel humps are more
/// than linear).
const MAX_CAMEL_HUMP_MATCHING_LENGTH: usize = 100;

/// Fragments collected from the end of the name to its start.
type Reversed = Vec<Range<usize>>;

#[derive(Debug, Clone)]
pub(crate) struct Minuscule {
    pattern: Vec<char>,
    is_lower: Vec<bool>,
    is_upper: Vec<bool>,
    is_word_separator: Vec<bool>,
    to_upper: Vec<char>,
    to_lower: Vec<char>,
    hard_separators: Vec<char>,
    mode: MatchingMode,
    mixed_case: bool,
    has_separators: bool,
    has_dots: bool,
    /// Each non-wildcard pattern char in lower and upper case.
    meaningful: Vec<char>,
}

fn is_wildcard_char(c: char) -> bool {
    c == ' ' || c == '*'
}

fn is_word_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '_' | '-' | ':' | '+' | '.')
}

impl Minuscule {
    pub(crate) fn new(pattern: &str, mode: MatchingMode, hard_separators: &str) -> Self {
        let pattern: Vec<char> = pattern
            .strip_suffix("* ")
            .unwrap_or(pattern)
            .chars()
            .collect();
        let n = pattern.len();
        let mut m = Self {
            is_lower: vec![false; n],
            is_upper: vec![false; n],
            is_word_separator: vec![false; n],
            to_upper: vec![' '; n],
            to_lower: vec![' '; n],
            hard_separators: hard_separators.chars().collect(),
            mode,
            mixed_case: false,
            has_separators: false,
            has_dots: false,
            meaningful: Vec::new(),
            pattern,
        };
        let mut seen_non_wildcard = false;
        let mut seen_lower = false;
        let mut seen_upper_not_after_wildcard = false;
        for k in 0..n {
            let c = m.pattern[k];
            let separator = is_word_separator(c);
            let up = is_upper(c);
            let low = is_lower(c);
            if low {
                seen_lower = true;
            }
            if c == '.' {
                m.has_dots = true;
            }
            if seen_non_wildcard && up {
                seen_upper_not_after_wildcard = true;
            }
            if !is_wildcard_char(c) {
                seen_non_wildcard = true;
                m.meaningful.push(lower(c));
                m.meaningful.push(upper(c));
            }
            if seen_non_wildcard && separator {
                m.has_separators = true;
            }
            m.is_word_separator[k] = separator;
            m.is_upper[k] = up;
            m.is_lower[k] = low;
            m.to_upper[k] = upper(c);
            m.to_lower[k] = lower(c);
        }
        m.mixed_case = seen_lower && seen_upper_not_after_wildcard;
        m
    }

    pub(crate) fn pattern(&self) -> &[char] {
        &self.pattern
    }

    /// The fragments of `name` the pattern matches, in order; `None` when it
    /// does not match.
    pub(crate) fn find(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        if name.len() < self.meaningful.len() / 2 {
            return None;
        }
        if self.pattern.len() > MAX_CAMEL_HUMP_MATCHING_LENGTH {
            return self.match_by_substring(name);
        }
        if !contains_meaningful_in_order(name, &self.meaningful) {
            return None;
        }
        let mut fragments = self.match_wildcards(name, 0, Some(0))?;
        fragments.reverse();
        Some(fragments)
    }

    /// How well the pattern matches, from its fragments (higher is
    /// better); `i32::MIN` without a match.
    pub(crate) fn degree(
        &self,
        name: &[char],
        value_start_case_match: bool,
        fragments: Option<&[Range<usize>]>,
    ) -> i32 {
        humped_score(self, name, value_start_case_match, fragments)
    }

    fn match_by_substring(&self, name: &[char]) -> Option<Vec<Range<usize>>> {
        let count = self.meaningful.len() / 2;
        if name.len() < count {
            return None;
        }
        if self.is_pattern_char(0, '*') {
            for i in 0..=name.len() - count {
                let len = self.meaningful_chars_match_at(name, i);
                if len != 0 {
                    return Some(vec![i..i + len]);
                }
            }
            return None;
        }
        let len = self.meaningful_chars_match_at(name, 0);
        (len != 0).then(|| vec![0..len])
    }

    fn meaningful_chars_match_at(&self, name: &[char], name_index: usize) -> usize {
        let mut k = 0;
        let mut pos = name_index;
        while pos < name.len() && k + 1 < self.meaningful.len() {
            let c = name[pos];
            if c == self.meaningful[k] || c == self.meaningful[k + 1] {
                k += 2;
                pos += 1;
            } else if is_wildcard_char(c) {
                pos += 1;
            } else {
                return 0;
            }
        }
        pos - name_index
    }

    /// After a wildcard, the next non-wildcard pattern char is looked for
    /// from `name_index` on.
    fn match_wildcards(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: Option<usize>,
    ) -> Option<Reversed> {
        let name_index = name_index?;
        let mut pi = pattern_index;
        if !self.is_wildcard(pi) {
            return if pi == self.pattern.len() {
                Some(Vec::new())
            } else {
                self.match_fragment(name, pi, name_index)
            };
        }
        loop {
            pi += 1;
            if !self.is_wildcard(pi) {
                break;
            }
        }
        if pi == self.pattern.len() {
            // A trailing space matches when the pattern ends with the last
            // word of the name, or only its first hump char.
            if self.is_trailing_space_pattern()
                && name_index != name.len()
                && (pi < 2 || !self.is_upper_case_or_digit(pi - 2))
            {
                let space = name[name_index..].iter().position(|&c| c == ' ')?;
                let at = name_index + space;
                return Some(vec![at..at + 1]);
            }
            return Some(Vec::new());
        }
        let next = self.find_next_pattern_char_occurrence(name, name_index, pi);
        self.match_skipping_words(name, pi, next, true)
    }

    fn is_trailing_space_pattern(&self) -> bool {
        self.pattern.last() == Some(&' ')
    }

    fn is_upper_case_or_digit(&self, pi: usize) -> bool {
        self.is_upper[pi] || is_digit(self.pattern[pi])
    }

    /// Tries the places in the name where the pattern char at
    /// `pattern_index` could match.
    fn match_skipping_words(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: Option<usize>,
        allow_special_chars: bool,
    ) -> Option<Reversed> {
        let mut ni = name_index;
        let mut max_found = 0;
        while let Some(n) = ni {
            let fragment = if self.seems_like_fragment_start(name, pattern_index, n) {
                self.max_matching_fragment(name, pattern_index, n)
            } else {
                0
            };
            // A fragment no longer than one already tried would fail again.
            if fragment > max_found
                || n + fragment == name.len() && self.is_trailing_space_pattern()
            {
                if !self.is_middle_match(name, pattern_index, n) {
                    max_found = fragment;
                }
                if let Some(ranges) = self.match_inside_fragment(name, pattern_index, n, fragment) {
                    return Some(ranges);
                }
            }
            let next = self.find_next_pattern_char_occurrence(name, n + 1, pattern_index);
            ni = if allow_special_chars {
                next
            } else {
                self.check_for_special_chars(name, n + 1, next, pattern_index)
            };
        }
        None
    }

    fn find_next_pattern_char_occurrence(
        &self,
        name: &[char],
        start_at: usize,
        pattern_index: usize,
    ) -> Option<usize> {
        if !self.is_pattern_char_before(pattern_index, '*')
            && !self.is_word_separator[pattern_index]
        {
            self.index_of_word_start(name, pattern_index, start_at)
        } else {
            self.index_of_ignore_case(name, start_at, pattern_index)
        }
    }

    fn check_for_special_chars(
        &self,
        name: &[char],
        start: usize,
        end: Option<usize>,
        pattern_index: usize,
    ) -> Option<usize> {
        let end = end?;
        let between = name.get(start..end).unwrap_or(&[]);
        // Pattern humps may match in words separated by the hard
        // separators, lower-case chars may not.
        if !self.has_separators
            && !self.mixed_case
            && between.iter().any(|c| self.hard_separators.contains(c))
        {
            return None;
        }
        // A typed dot does not skip other dots between humps (one pattern
        // dot may match several name dots).
        if self.has_dots
            && !self.is_pattern_char_before(pattern_index, '.')
            && between.contains(&'.')
        {
            return None;
        }
        Some(end)
    }

    fn seems_like_fragment_start(&self, name: &[char], pattern_index: usize, at: usize) -> bool {
        // An upper-case pattern char matches an upper-case char or a word
        // start, or anything when the pattern is not mixed-case.
        !self.is_upper[pattern_index]
            || is_upper(name[at])
            || is_word_start(name, at)
            || !self.mixed_case && self.mode != MatchingMode::MatchCase
    }

    fn char_equals(
        &self,
        pattern_char: char,
        pattern_index: usize,
        c: char,
        ignore_case: bool,
    ) -> bool {
        pattern_char == c
            || ignore_case
                && (self.to_lower[pattern_index] == c || self.to_upper[pattern_index] == c)
    }

    fn match_fragment(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: usize,
    ) -> Option<Reversed> {
        let fragment = self.max_matching_fragment(name, pattern_index, name_index);
        if fragment == 0 {
            None
        } else {
            self.match_inside_fragment(name, pattern_index, name_index, fragment)
        }
    }

    fn max_matching_fragment(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: usize,
    ) -> usize {
        if !self.is_first_char_matching(name, name_index, pattern_index) {
            return 0;
        }
        let ignore_case = self.mode != MatchingMode::MatchCase;
        let mut i = 1;
        while name_index + i < name.len() && pattern_index + i < self.pattern.len() {
            let c = name[name_index + i];
            if !self.char_equals(
                self.pattern[pattern_index + i],
                pattern_index + i,
                c,
                ignore_case,
            ) {
                if self.is_skipping_digit_between_pattern_digits(pattern_index + i, c) {
                    return 0;
                }
                break;
            }
            i += 1;
        }
        i
    }

    fn is_skipping_digit_between_pattern_digits(&self, pattern_index: usize, c: char) -> bool {
        is_digit(self.pattern[pattern_index])
            && is_digit(self.pattern[pattern_index - 1])
            && is_digit(c)
    }

    /// The longest fragment matches: camel humps further on are tried
    /// first, then shorter prefixes of the fragment.
    fn match_inside_fragment(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: usize,
        fragment: usize,
    ) -> Option<Reversed> {
        // An exact match in the middle of a word takes at least 3 chars.
        let min_fragment = if self.is_middle_match(name, pattern_index, name_index) {
            3
        } else {
            1
        };
        self.improve_camel_humps(name, pattern_index, name_index, fragment, min_fragment)
            .or_else(|| {
                self.find_longest_matching_prefix(
                    name,
                    pattern_index,
                    name_index,
                    fragment,
                    min_fragment,
                )
            })
    }

    fn is_middle_match(&self, name: &[char], pattern_index: usize, name_index: usize) -> bool {
        self.is_pattern_char_before(pattern_index, '*')
            && !self.is_wildcard(pattern_index + 1)
            && is_letter_or_digit(name[name_index])
            && !is_word_start(name, name_index)
    }

    fn find_longest_matching_prefix(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: usize,
        fragment: usize,
        min_fragment: usize,
    ) -> Option<Reversed> {
        if pattern_index + fragment >= self.pattern.len() {
            return Some(vec![name_index..name_index + fragment]);
        }
        // The rest of the pattern against the rest of the name, with the
        // longest fragment first, then shorter ones.
        let mut i = fragment;
        while i >= min_fragment || i > 0 && self.is_wildcard(pattern_index + i) {
            let ranges = if self.is_wildcard(pattern_index + i) {
                self.match_wildcards(name, pattern_index + i, Some(name_index + i))
            } else {
                let next = self.find_next_pattern_char_occurrence(
                    name,
                    name_index + i + 1,
                    pattern_index + i,
                );
                match self.check_for_special_chars(name, name_index + i, next, pattern_index + i) {
                    Some(n) => self.match_skipping_words(name, pattern_index + i, Some(n), false),
                    None => None,
                }
            };
            if let Some(ranges) = ranges {
                return Some(append_range(ranges, name_index, i));
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        None
    }

    /// For "CU" and "CurrentUser" the prefix "Cu" matches, but the "U"
    /// further on matches better.
    fn improve_camel_humps(
        &self,
        name: &[char],
        pattern_index: usize,
        name_index: usize,
        max_fragment: usize,
        min_fragment: usize,
    ) -> Option<Reversed> {
        for i in min_fragment..max_fragment {
            if self.is_upper[pattern_index + i]
                && self.pattern[pattern_index + i] != name[name_index + i]
            {
                let next = self.index_of_word_start(name, pattern_index + i, name_index + i);
                if let Some(ranges) = self.match_wildcards(name, pattern_index + i, next) {
                    return Some(append_range(ranges, name_index, i));
                }
            }
        }
        None
    }

    fn is_first_char_matching(
        &self,
        name: &[char],
        name_index: usize,
        pattern_index: usize,
    ) -> bool {
        if name_index >= name.len() {
            return false;
        }
        let ignore_case = self.mode != MatchingMode::MatchCase;
        if !self.char_equals(
            self.pattern[pattern_index],
            pattern_index,
            name[name_index],
            ignore_case,
        ) {
            return false;
        }
        !(self.mode == MatchingMode::FirstLetter
            && (pattern_index == 0 || pattern_index == 1 && self.is_wildcard(0))
            && (self.is_upper[pattern_index] || self.is_lower[pattern_index])
            && self.is_upper[pattern_index] != is_upper(name[0]))
    }

    fn is_wildcard(&self, pattern_index: usize) -> bool {
        self.pattern
            .get(pattern_index)
            .is_some_and(|&c| is_wildcard_char(c))
    }

    fn is_pattern_char(&self, pattern_index: usize, c: char) -> bool {
        self.pattern.get(pattern_index) == Some(&c)
    }

    /// The pattern char before `pattern_index` is `c`.
    fn is_pattern_char_before(&self, pattern_index: usize, c: char) -> bool {
        pattern_index > 0 && self.is_pattern_char(pattern_index - 1, c)
    }

    fn index_of_word_start(
        &self,
        name: &[char],
        pattern_index: usize,
        start_from: usize,
    ) -> Option<usize> {
        let p = self.pattern[pattern_index];
        if start_from >= name.len()
            || self.mixed_case
                && self.is_lower[pattern_index]
                && !(pattern_index > 0 && self.is_word_separator[pattern_index - 1])
        {
            return None;
        }
        let special = !is_letter_or_digit(p);
        let mut i = start_from;
        loop {
            i = self.index_of_ignore_case(name, i, pattern_index)?;
            if special || is_word_start(name, i) {
                return Some(i);
            }
            i += 1;
        }
    }

    fn index_of_ignore_case(
        &self,
        name: &[char],
        from: usize,
        pattern_index: usize,
    ) -> Option<usize> {
        let p = self.pattern[pattern_index];
        let tail = name.get(from..)?;
        if p.is_ascii() {
            let (pu, pl) = (self.to_upper[pattern_index], self.to_lower[pattern_index]);
            tail.iter()
                .position(|&c| c == pu || c == pl)
                .map(|i| from + i)
        } else {
            tail.iter()
                .position(|&c| eq_ignore_case(c, p))
                .map(|i| from + i)
        }
    }
}

/// Adds the fragment `from..from + length` before the ones found after it
/// (merged with the first of them when they touch).
fn append_range(mut ranges: Reversed, from: usize, length: usize) -> Reversed {
    match ranges.last_mut() {
        Some(last) if last.start == from + length => last.start = from,
        _ => ranges.push(from..from + length),
    }
    ranges
}

/// Every meaningful pattern char occurs in the name in order (either case).
fn contains_meaningful_in_order(name: &[char], meaningful: &[char]) -> bool {
    let index_of = |c: char, from: usize| {
        name.get(from..)?
            .iter()
            .position(|&x| x == c)
            .map(|i| from + i)
    };
    let mut k = 0;
    let mut ni = 0;
    while k + 1 < meaningful.len() {
        if ni >= name.len() {
            return false;
        }
        let c1 = meaningful[k];
        let i1 = index_of(c1, ni);
        if i1 == Some(ni) {
            ni += 1;
        } else {
            let c2 = meaningful[k + 1];
            if c1 == c2 {
                match i1 {
                    Some(a) => ni = a + 1,
                    None => return false,
                }
            } else {
                ni = match (i1, index_of(c2, ni)) {
                    (Some(a), Some(b)) => a.min(b) + 1,
                    (Some(a), None) => a + 1,
                    (None, Some(b)) => b + 1,
                    (None, None) => return false,
                };
            }
        }
        k += 2;
    }
    true
}

/// The reference's matching degree: a match at a word start, matching
/// case (especially a typed capital), few fragments, no skipped humps, the
/// start and the end of the name all count.
fn humped_score(
    m: &Minuscule,
    name: &[char],
    value_start_case_match: bool,
    fragments: Option<&[Range<usize>]>,
) -> i32 {
    let Some(fragments) = fragments else {
        return i32::MIN;
    };
    let Some(first) = fragments.first() else {
        return 0;
    };
    let pattern = &m.pattern;
    let start_match = first.start == 0;
    let valued_start_match = start_match && value_start_case_match;
    let mut matching_case: i32 = 0;
    let mut p: Option<usize> = None;
    let mut skipped_humps: i32 = 0;
    let mut next_hump_start = 0;
    let mut hump_start_matched_upper = false;
    let mut prev_end = first.start;
    for (k, range) in fragments.iter().enumerate() {
        skipped_humps += count_skipped_upper_case_inside_word(name, prev_end, range.start);
        prev_end = range.end;
        for i in range.clone() {
            let after_gap = i == range.start && k > 0;
            let mut is_hump_start = false;
            while next_hump_start <= i {
                if next_hump_start == i {
                    is_hump_start = true;
                } else if after_gap {
                    skipped_humps += 1;
                }
                next_hump_start = if next_hump_start < name.len() && is_digit(name[next_hump_start])
                {
                    next_hump_start + 1
                } else {
                    next_word(name, next_hump_start)
                };
            }
            let c = name[i];
            let from = p.map_or(0, |p| p + 1);
            // Not in the rest of the pattern: the next fragment looks from
            // its start again.
            let Some(found) = (from..pattern.len()).find(|&j| eq_ignore_case(pattern[j], c)) else {
                p = None;
                break;
            };
            p = Some(found);
            if is_hump_start {
                hump_start_matched_upper = c == pattern[found] && m.is_upper[found];
            }
            matching_case += case_matching(
                m,
                valued_start_match,
                found,
                hump_start_matched_upper,
                i,
                after_gap,
                is_hump_start,
                c,
            );
        }
    }
    let start = first.start;
    let after_separator = name[..start].iter().any(|c| m.hard_separators.contains(c));
    let word_start = start == 0 || is_word_start(name, start) && !is_word_start(name, start - 1);
    let final_match = fragments.last().is_some_and(|f| f.end == name.len());
    (if word_start { 1000 } else { 0 }) + matching_case
        - fragments.len() as i32
        - skipped_humps * 10
        + if after_separator { 0 } else { 2 }
        + i32::from(start_match)
        + i32::from(final_match)
}

/// Capitals in `start..end` that are not word starts: skipped parts of an
/// all-caps word (the `A` and the `O` in `DAOHolder`).
fn count_skipped_upper_case_inside_word(name: &[char], start: usize, end: usize) -> i32 {
    (start..end)
        .filter(|&i| is_upper(name[i]) && !is_word_start(name, i))
        .count() as i32
}

#[allow(clippy::too_many_arguments)]
fn case_matching(
    m: &Minuscule,
    valued_start_match: bool,
    pattern_index: usize,
    hump_start_matched_upper: bool,
    name_index: usize,
    after_gap: bool,
    is_hump_start: bool,
    name_char: char,
) -> i32 {
    if after_gap && is_hump_start && m.is_lower[pattern_index] {
        // A hump the pattern gave no sign of.
        return -10;
    }
    if name_char == m.pattern[pattern_index] {
        if m.is_upper[pattern_index] {
            // A typed capital matching a capital.
            return 50;
        }
        if name_index == 0 && valued_start_match {
            return 150;
        }
        return i32::from(is_hump_start);
    }
    if is_hump_start {
        return -1;
    }
    if m.is_lower[pattern_index] && hump_start_matched_upper {
        return -1;
    }
    0
}
