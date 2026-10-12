//! Byte offsets of matches to char offsets of the text model.

use std::ops::Range;

/// Converts sorted, non-overlapping byte ranges of `haystack` (matches in
/// the order a search found them) to char ranges, in place, in one pass
/// over the text.
pub fn to_char_ranges(haystack: &str, ranges: &mut [Range<usize>]) {
    let (mut byte, mut chars) = (0usize, 0usize);
    for r in ranges {
        debug_assert!(r.start >= byte && r.end >= r.start, "unsorted ranges");
        chars += haystack[byte..r.start].chars().count();
        let start = chars;
        chars += haystack[r.start..r.end].chars().count();
        byte = r.end;
        *r = start..chars;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ranges_after_multibyte_text() {
        let text = "ä😀 ab ü ab";
        let a = text.find("ab").unwrap();
        let b = text.rfind("ab").unwrap();
        let mut ranges = vec![a..a + 2, b..b + 2, text.len()..text.len()];
        to_char_ranges(text, &mut ranges);
        assert_eq!(ranges, vec![3..5, 8..10, 10..10]);
    }
}
