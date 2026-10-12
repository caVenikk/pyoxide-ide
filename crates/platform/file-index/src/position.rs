//! A line and column after the name: `app.py:12`, `app.py:12:5`,
//! `app.py(12)`, `app.py 12`, `app.py#L12`, `app.py at line 12`, as the
//! reference's Go to File accepts them.

use std::sync::LazyLock;

use regex::Regex;

/// The name (lazy), a separator, the line and column digits, a closing
/// bracket.
static LINE_AND_COLUMN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+?)(?::|@|,| |#|#L|\?l=| on line | at line |:line |:?\(|:?\[)(\d+)?(?:\W(\d+)?)?[)\]]?$")
        .expect("valid pattern")
});

/// A search text split into the name pattern and the place to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    /// The text without the line and column.
    pub pattern: String,
    /// Zero-based line, when one was given (0 when only a column was).
    pub line: Option<usize>,
    /// Zero-based column.
    pub column: Option<usize>,
}

/// Splits `text` as the reference does: the line and column are taken
/// only when the whole text has the form `name<separator>line[<non-word
/// char>column]`; a trailing space is kept on the pattern (it asks for a
/// whole-word end).
pub fn parse_position(text: &str) -> Position {
    let quick = text.contains([':', ',', ';', '@', '[', '(', ' ', '#'])
        || text.contains(" line ")
        || text.contains("?l=");
    let caps = quick.then(|| LINE_AND_COLUMN.captures(text)).flatten();
    let Some(caps) = caps else {
        return Position {
            pattern: text.to_owned(),
            line: None,
            column: None,
        };
    };
    let number = |i: usize| {
        caps.get(i)
            .and_then(|m| m.as_str().parse::<usize>().ok())
            .map(|n| n.saturating_sub(1))
    };
    let mut pattern = caps.get(1).map_or("", |m| m.as_str()).to_owned();
    if text.ends_with(' ') {
        pattern.push(' ');
    }
    let (line, column) = (number(2), number(3));
    Position {
        pattern,
        line: line.or(column.map(|_| 0)),
        column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> (String, Option<usize>, Option<usize>) {
        let p = parse_position(text);
        (p.pattern, p.line, p.column)
    }

    #[test]
    fn lines_and_columns() {
        assert_eq!(at("app.py"), ("app.py".into(), None, None));
        assert_eq!(at("app.py:12"), ("app.py".into(), Some(11), None));
        assert_eq!(at("app.py:12:5"), ("app.py".into(), Some(11), Some(4)));
        assert_eq!(at("app.py:12,5"), ("app.py".into(), Some(11), Some(4)));
        assert_eq!(at("app.py(12)"), ("app.py".into(), Some(11), None));
        // The first separator that fits wins: `:`, then `(` before the
        // column (a line 0).
        assert_eq!(at("app.py:(12)"), ("app.py".into(), Some(0), Some(11)));
        assert_eq!(at("app.py[12:5]"), ("app.py".into(), Some(11), Some(4)));
        assert_eq!(at("app.py 12"), ("app.py".into(), Some(11), None));
        assert_eq!(at("app.py#L12"), ("app.py".into(), Some(11), None));
        assert_eq!(at("app.py at line 7"), ("app.py".into(), Some(6), None));
        assert_eq!(at("app.py:"), ("app.py".into(), None, None));
        assert_eq!(at("src/app.py:3"), ("src/app.py".into(), Some(2), None));
    }

    #[test]
    fn other_texts_stay_as_they_are() {
        assert_eq!(at("flask app"), ("flask app".into(), None, None));
        assert_eq!(at("app "), ("app ".into(), None, None));
        assert_eq!(at("a:b"), ("a:b".into(), None, None));
    }
}
