mod candidates;
mod classify;
mod dialect;
mod explain;
mod lexer;
mod params;
mod soup;
mod split;

use sql_syntax::{Dialect, Statement};

pub fn texts(script: &str, dialect: Dialect) -> Vec<String> {
    sql_syntax::split(script, dialect)
        .into_iter()
        .map(|s| s.text)
        .collect()
}

/// The statements are ordered, do not overlap, lie on character boundaries and
/// their text is the text of their range.
pub fn check_ranges(script: &str, statements: &[Statement]) {
    let mut end = 0;
    for s in statements {
        assert!(
            s.range.start >= end,
            "overlap in {script:?}: {statements:?}"
        );
        assert!(s.range.start < s.range.end);
        assert!(script.is_char_boundary(s.range.start) && script.is_char_boundary(s.range.end));
        assert_eq!(&script[s.range.clone()], s.text);
        end = s.range.end;
    }
}

/// A small deterministic random generator (xorshift): the property tests use
/// fixed seeds, so a failure repeats.
pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}
