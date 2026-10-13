//! Character classes and the word structure of names: where a word (a
//! "hump") starts and where it ends, as the reference IDE splits
//! `NameUtilTest`, `name_util_test` or `HTMLParser2`.

pub(crate) fn is_upper(c: char) -> bool {
    c.is_uppercase()
}

pub(crate) fn is_lower(c: char) -> bool {
    c.is_lowercase()
}

pub(crate) fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

pub(crate) fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric())
}

pub(crate) fn is_letter_or_digit(c: char) -> bool {
    is_letter(c) || is_digit(c)
}

/// The single-char upper case of `c` (`c` itself when it has none or more
/// than one char).
pub(crate) fn upper(c: char) -> char {
    single(c.to_uppercase()).unwrap_or(c)
}

pub(crate) fn lower(c: char) -> char {
    single(c.to_lowercase()).unwrap_or(c)
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

/// Equal ignoring case, as `Char.equals(other, ignoreCase = true)`.
pub(crate) fn eq_ignore_case(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let (ua, ub) = (upper(a), upper(b));
    ua == ub || lower(ua) == lower(ub)
}

/// CJK ideographs: each one is a word of its own.
fn is_ideographic(c: char) -> bool {
    matches!(c as u32,
        0x3006 | 0x3007 | 0x3021..=0x3029 | 0x3038..=0x303A
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF)
}

/// A word starts at `i`: an upper-case letter after a lower-case one (or
/// the last capital of an all-caps word before a lower-case letter), a
/// digit, or a letter after something that is not a letter or digit.
pub fn is_word_start(text: &[char], i: usize) -> bool {
    let cur = text[i];
    let prev = (i > 0).then(|| text[i - 1]);
    if is_upper(cur) {
        if prev.is_some_and(is_upper) {
            // Not in the middle of an all-caps word.
            return text.get(i + 1).is_some_and(|&c| is_lower(c));
        }
        return true;
    }
    if is_digit(cur) {
        return true;
    }
    if !is_letter(cur) {
        return false;
    }
    if is_ideographic(cur) {
        return true;
    }
    i == 0 || !is_letter_or_digit(text[i - 1])
}

/// Where the word that starts at `start` ends: digits form a word, several
/// capitals form one (the last of them starts the next word when a
/// lower-case letter follows), a non-letter is a word of its own.
pub fn next_word(text: &[char], start: usize) -> usize {
    let ch = text[start];
    if !is_letter_or_digit(ch) {
        return start + 1;
    }
    let mut i = start;
    while i < text.len() && is_digit(text[i]) {
        i += 1;
    }
    if i > start {
        return i;
    }
    while i < text.len() && is_upper(text[i]) {
        i += 1;
    }
    if i > start + 1 {
        if i == text.len() || !is_letter(text[i]) {
            return i;
        }
        return i - 1;
    }
    if i == start {
        i += 1;
    }
    while i < text.len() && is_letter(text[i]) && !is_word_start(text, i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        let t: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < t.len() {
            let end = next_word(&t, i);
            out.push(t[i..end].iter().collect());
            i = end;
        }
        out
    }

    #[test]
    fn names_split_into_words() {
        assert_eq!(words("NameUtilTest"), ["Name", "Util", "Test"]);
        assert_eq!(words("name_util"), ["name", "_", "util"]);
        assert_eq!(words("HTMLParser2"), ["HTML", "Parser", "2"]);
        assert_eq!(words("app.py"), ["app", ".", "py"]);
        assert_eq!(words("DAOHolder"), ["DAO", "Holder"]);
    }

    #[test]
    fn word_starts() {
        let t: Vec<char> = "getURLFor x_y".chars().collect();
        let starts: Vec<usize> = (0..t.len()).filter(|&i| is_word_start(&t, i)).collect();
        // g, U (after lower), F (capital before lower), x, y.
        assert_eq!(starts, [0, 3, 6, 10, 12]);
    }
}
