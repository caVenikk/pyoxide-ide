//! A pattern typed in the wrong keyboard layout: the reference IDE also
//! tries the letters on the same keys of the English layout, so `ыек`
//! (typed with the Russian layout on) finds `String`.

/// The Russian (ЙЦУКЕН) letters and the English keys they share.
const RUSSIAN: [(char, char); 33] = [
    ('й', 'q'),
    ('ц', 'w'),
    ('у', 'e'),
    ('к', 'r'),
    ('е', 't'),
    ('н', 'y'),
    ('г', 'u'),
    ('ш', 'i'),
    ('щ', 'o'),
    ('з', 'p'),
    ('х', '['),
    ('ъ', ']'),
    ('ф', 'a'),
    ('ы', 's'),
    ('в', 'd'),
    ('а', 'f'),
    ('п', 'g'),
    ('р', 'h'),
    ('о', 'j'),
    ('л', 'k'),
    ('д', 'l'),
    ('ж', ';'),
    ('э', '\''),
    ('я', 'z'),
    ('ч', 'x'),
    ('с', 'c'),
    ('м', 'v'),
    ('и', 'b'),
    ('т', 'n'),
    ('ь', 'm'),
    ('б', ','),
    ('ю', '.'),
    ('ё', '`'),
];

/// The English key of a letter of the Russian layout (its case kept).
fn ascii_for(c: char) -> Option<char> {
    let lc = super::words::lower(c);
    let (_, ascii) = RUSSIAN.iter().find(|(r, _)| *r == lc)?;
    Some(if lc == c {
        *ascii
    } else {
        super::words::upper(*ascii)
    })
}

/// The pattern with its letters on the English keys, when every letter in
/// it is a non-ASCII one (else `None`).
pub fn fix_layout(pattern: &str) -> Option<String> {
    let mut has_letters = false;
    for c in pattern.chars() {
        if super::words::is_letter(c) {
            has_letters = true;
            if c.is_ascii() {
                return None;
            }
        }
    }
    has_letters.then(|| pattern.chars().map(|c| ascii_for(c).unwrap_or(c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_on_the_same_keys() {
        assert_eq!(fix_layout("ыек").as_deref(), Some("str"));
        assert_eq!(fix_layout("Ыек").as_deref(), Some("Str"));
        assert_eq!(fix_layout("ыеr"), None, "an ASCII letter: typed on purpose");
        assert_eq!(fix_layout("123"), None);
    }
}
