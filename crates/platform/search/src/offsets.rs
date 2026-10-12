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

/// The inverse of [`to_char_ranges`]: sorted, non-overlapping char ranges
/// of `haystack` to byte ranges, in place, in one pass. Ranges past the end
/// are clamped to it.
pub fn to_byte_ranges(haystack: &str, ranges: &mut [Range<usize>]) {
    let (mut chars, mut byte) = (0usize, 0usize);
    let mut advance = |to: usize| -> usize {
        debug_assert!(to >= chars, "unsorted ranges");
        let rest = &haystack[byte..];
        byte += rest
            .char_indices()
            .nth(to - chars)
            .map_or(rest.len(), |(i, _)| i);
        chars = to;
        byte
    };
    for r in ranges {
        let start = advance(r.start);
        let end = advance(r.end);
        *r = start..end;
    }
}

/// Zero-based lines where the matches (sorted byte ranges of `haystack`)
/// start, each line once; for marks on a scrollbar or a list of results.
pub fn match_lines(haystack: &str, ranges: &[Range<usize>]) -> Vec<usize> {
    let bytes = haystack.as_bytes();
    let mut lines: Vec<usize> = Vec::new();
    let (mut byte, mut line) = (0usize, 0usize);
    for r in ranges {
        line += bytes[byte..r.start].iter().filter(|&&b| b == b'\n').count();
        byte = r.start;
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // lists of ranges
mod tests {
    use super::*;

    #[test]
    fn converts_ranges_after_multibyte_text() {
        let text = "ä😀 ab ü ab";
        let a = text.find("ab").unwrap();
        let b = text.rfind("ab").unwrap();
        let mut ranges = vec![a..a + 2, b..b + 2, text.len()..text.len()];
        let bytes = ranges.clone();
        to_char_ranges(text, &mut ranges);
        assert_eq!(ranges, vec![3..5, 8..10, 10..10]);
        to_byte_ranges(text, &mut ranges);
        assert_eq!(ranges, bytes);
        let mut past = vec![10..12];
        to_byte_ranges(text, &mut past);
        assert_eq!(past, vec![text.len()..text.len()]);
    }

    #[test]
    fn lists_each_matching_line_once() {
        let text = "ab ab\ncd\nab\n\nab";
        let ranges: Vec<Range<usize>> = text.match_indices("ab").map(|(i, _)| i..i + 2).collect();
        assert_eq!(match_lines(text, &ranges), vec![0, 2, 4]);
        assert!(match_lines(text, &[]).is_empty());
    }
}
