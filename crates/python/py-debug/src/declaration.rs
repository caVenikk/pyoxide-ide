//! Jump To Source of a frame's value (`PyDebugValue.computeSourcePosition`,
//! `PyDebugProcess.getSourcePositionForName`, which resolves the name in
//! the scope of the frame's line with `PyResolveUtil.scopeCrawlUp`): the
//! first place in the scope of the frame's function, from its `def` line,
//! where the name stands as a word of code (a parameter, the target of an
//! assignment, of a `for`, of an `import`, of a `with ... as`, ...); for a
//! frame on the module level the first such place on the module level.
//! The bodies of nested functions and classes are not the scope's, their
//! names are. Without the reference's resolver, a word of code is any
//! identifier outside strings and comments that does not follow a dot.
//! Lines and columns count from 1.

use crate::lines::classify;

/// The line and column of the declaration of `name` in the scope of a
/// frame suspended on `line` of `text`.
pub(crate) fn declaration(text: &str, line: u32, name: &str) -> Option<(u32, u32)> {
    if !is_identifier(name) {
        return None;
    }
    let lines = Lines::new(text);
    let frame = (line as usize).saturating_sub(1).min(lines.count() - 1);
    // The innermost function or class whose body holds the frame's line.
    let mut level = lines.indent(frame).unwrap_or(usize::MAX);
    let mut scope = None;
    for i in (0..frame).rev() {
        let Some(indent) = lines.indent(i) else {
            continue;
        };
        if indent < level {
            level = indent;
            if lines.header(i).is_some() {
                scope = Some(i);
                break;
            }
        }
        if level == 0 {
            break;
        }
    }
    let (first, header_indent) = match scope {
        Some(i) => (i, lines.indent(i)),
        None => (0, None),
    };
    // The nested block being skipped: the indentation of its header.
    let mut nested: Option<usize> = None;
    for i in first..lines.count() {
        let indent = lines.indent(i);
        if i > first
            && let (Some(indent), Some(header)) = (indent, header_indent)
            && indent <= header
        {
            break;
        }
        if let (Some(indent), Some(block)) = (indent, nested) {
            if indent > block {
                continue;
            }
            nested = None;
        }
        let mut from = lines.start(i);
        if lines.header(i).is_some() && (i > first || scope.is_none()) {
            // The name of a nested function or class is the scope's, its
            // body is not.
            nested = indent;
            if let Some((word, at)) = lines.words(i, from).next()
                && word == name
            {
                return Some(lines.position(i, at));
            }
            continue;
        }
        if i == first && scope.is_some() {
            // The scope's own name is the enclosing scope's: its
            // parameters are the scope's.
            if let Some((_, at)) = lines.words(i, from).next() {
                from = at + 1;
            }
        }
        if let Some((_, at)) = lines.words(i, from).find(|(w, _)| *w == name) {
            return Some(lines.position(i, at));
        }
    }
    None
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// The lines of a text with the reference lexer's code bytes.
struct Lines<'a> {
    text: &'a str,
    code: Vec<bool>,
    /// Byte offsets of the line starts.
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self {
            text,
            code: classify(text.as_bytes()),
            starts,
        }
    }

    fn count(&self) -> usize {
        self.starts.len()
    }

    fn start(&self, line: usize) -> usize {
        self.starts[line]
    }

    fn end(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1)
    }

    /// The offset of the line's first code byte.
    fn first_code(&self, line: usize) -> Option<usize> {
        (self.start(line)..self.end(line)).find(|&i| self.code[i])
    }

    /// The indentation of a line with code, in bytes.
    fn indent(&self, line: usize) -> Option<usize> {
        self.first_code(line).map(|i| i - self.start(line))
    }

    /// `def` (`async def`) or `class` when the line starts a block of one.
    fn header(&self, line: usize) -> Option<&'static str> {
        let first = self.first_code(line)?;
        let rest = &self.text[first..self.end(line)];
        let rest = rest
            .strip_prefix("async")
            .filter(|r| r.starts_with([' ', '\t']))
            .map_or(rest, str::trim_start);
        let keyword = |k: &str| {
            rest.strip_prefix(k)
                .is_some_and(|r| r.starts_with([' ', '\t']))
        };
        if keyword("def") {
            Some("def")
        } else if keyword("class") {
            Some("class")
        } else {
            None
        }
    }

    /// The words of code of a line from an offset that do not follow a
    /// dot (attributes), with their offsets; keywords included.
    fn words(&self, line: usize, from: usize) -> impl Iterator<Item = (&'a str, usize)> + '_ {
        let end = self.end(line);
        let text = self.text;
        let code = &self.code;
        let mut i = from;
        std::iter::from_fn(move || {
            while i < end {
                let c = text[i..].chars().next()?;
                let len = c.len_utf8();
                if !(code[i] && (c == '_' || c.is_alphabetic())) {
                    i += len;
                    continue;
                }
                let start = i;
                while i < end {
                    let c = text[i..].chars().next()?;
                    if !(c == '_' || c.is_alphanumeric()) {
                        break;
                    }
                    i += c.len_utf8();
                }
                let before = text[..start].trim_end_matches([' ', '\t']);
                if before.ends_with('.') || is_keyword(&text[start..i]) {
                    continue;
                }
                return Some((&text[start..i], start));
            }
            None
        })
    }

    /// The line and column (characters, from 1) of an offset.
    fn position(&self, line: usize, offset: usize) -> (u32, u32) {
        let column = self.text[self.start(line)..offset].chars().count() + 1;
        (line as u32 + 1, column as u32)
    }
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "def"
            | "class"
            | "async"
            | "await"
            | "for"
            | "in"
            | "if"
            | "elif"
            | "else"
            | "while"
            | "with"
            | "as"
            | "import"
            | "from"
            | "return"
            | "yield"
            | "lambda"
            | "not"
            | "and"
            | "or"
            | "is"
            | "try"
            | "except"
            | "finally"
            | "raise"
            | "global"
            | "nonlocal"
            | "pass"
            | "del"
            | "assert"
            | "break"
            | "continue"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = r#"def describe(items):
    summary = {"count": len(items), "sum": 0}
    for value in items:
        summary["sum"] += value  # value
    return summary


def other(value):
    def inner(summary):
        return summary.value
    return inner(value)


numbers = [1, 2, 3, 4]
text = "numbers"
print(describe(numbers))
"#;

    #[test]
    fn names_resolve_in_the_frame_function() {
        assert_eq!(declaration(MAIN, 4, "items"), Some((1, 14)));
        assert_eq!(declaration(MAIN, 4, "summary"), Some((2, 5)));
        assert_eq!(declaration(MAIN, 4, "value"), Some((3, 9)));
        assert_eq!(declaration(MAIN, 4, "missing"), None);
        // The function's own name is not its local.
        assert_eq!(declaration(MAIN, 4, "describe"), None);
    }

    #[test]
    fn nested_bodies_are_skipped_but_their_names_count() {
        assert_eq!(declaration(MAIN, 11, "value"), Some((8, 11)));
        assert_eq!(declaration(MAIN, 11, "inner"), Some((9, 9)));
        assert_eq!(declaration(MAIN, 11, "summary"), None);
        assert_eq!(declaration(MAIN, 10, "summary"), Some((9, 15)));
    }

    #[test]
    fn module_names_skip_functions_strings_and_attributes() {
        assert_eq!(declaration(MAIN, 16, "numbers"), Some((14, 1)));
        assert_eq!(declaration(MAIN, 16, "describe"), Some((1, 5)));
        assert_eq!(declaration(MAIN, 16, "items"), None);
        assert_eq!(declaration(MAIN, 16, "text"), Some((15, 1)));
        assert_eq!(declaration(MAIN, 16, "not a name"), None);
    }
}
