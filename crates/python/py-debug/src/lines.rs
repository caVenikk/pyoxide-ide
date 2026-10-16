//! The lines a Python line breakpoint can be put on, by the reference's
//! rule (`PyLineBreakpointType.canPutAt`, `XDebuggerUtilImpl.iterateLine`,
//! `PyDebugSupportUtils.isContinuationLine`): going through the tokens of
//! the line from its start, whitespace, comments and string literals
//! (docstrings included) are skipped; any other token makes the line
//! stoppable. A line after one that ends with a backslash is not.
//!
//! The tokens are the reference lexer's (`Python.flex`): a token that
//! starts on an earlier line counts on every line it covers, so the lines
//! inside a triple-quoted string are not stoppable while a line that
//! continues after the string's end is; f-strings and t-strings are not
//! string literal tokens there (`FSTRING_START`, `FSTRING_TEXT`, ...), so a
//! line with f-string text is stoppable; lines inside brackets are
//! stoppable. Lines count from 1, as the debugger model counts them.

/// The stoppable lines of a file's text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoppableLines {
    /// By line, from line 1.
    stoppable: Vec<bool>,
}

impl StoppableLines {
    /// The stoppable lines of `text` (one pass over it).
    pub fn of(text: &str) -> Self {
        let bytes = text.as_bytes();
        let code = classify(bytes);
        let mut stoppable = Vec::new();
        let mut start = 0;
        let mut after_backslash = false;
        loop {
            let end = bytes[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |n| start + n);
            let has_code = code[start..end].iter().any(|&c| c);
            stoppable.push(has_code && !after_backslash);
            // `String.trim()`: everything up to a space counts as blank.
            let line = &bytes[start..end];
            let trimmed = match line.iter().rposition(|&b| b > b' ') {
                Some(last) => &line[..=last],
                None => &[][..],
            };
            after_backslash = trimmed.last() == Some(&b'\\');
            if end == bytes.len() {
                break;
            }
            start = end + 1;
        }
        Self { stoppable }
    }

    /// A line breakpoint can be put on `line` (from 1).
    pub fn contains(&self, line: u32) -> bool {
        line >= 1
            && self
                .stoppable
                .get(line as usize - 1)
                .copied()
                .unwrap_or(false)
    }

    /// The stoppable lines, from 1.
    pub fn lines(&self) -> impl Iterator<Item = u32> + '_ {
        self.stoppable
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .map(|(i, _)| i as u32 + 1)
    }

    /// The number of lines of the text.
    pub fn line_count(&self) -> usize {
        self.stoppable.len()
    }
}

/// Whether a line breakpoint can be put on `line` (from 1) of `text`.
pub fn is_stoppable(text: &str, line: u32) -> bool {
    StoppableLines::of(text).contains(line)
}

/// Marks the bytes of tokens that make a line stoppable.
fn classify(bytes: &[u8]) -> Vec<bool> {
    let mut lexer = Lexer {
        s: bytes,
        i: 0,
        code: vec![false; bytes.len()],
    };
    lexer.code(None);
    lexer.code
}

/// What ends a run of code tokens inside an f-string fragment.
#[derive(Clone, Copy)]
struct Fragment {
    /// The quote of the f-string.
    quote: u8,
    triple: bool,
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
    code: Vec<bool>,
}

impl Lexer<'_> {
    fn at(&self, offset: usize) -> Option<u8> {
        self.s.get(self.i + offset).copied()
    }

    fn mark(&mut self, from: usize, to: usize) {
        let to = to.min(self.s.len());
        for c in &mut self.code[from..to] {
            *c = true;
        }
    }

    /// Code tokens to the end of the text, or in a fragment (`{...}` of an
    /// f-string) to its closing brace. True when the text or the f-string
    /// ended first.
    fn code(&mut self, fragment: Option<Fragment>) -> bool {
        let mut depth = 0usize;
        while let Some(b) = self.at(0) {
            match b {
                b' ' | b'\t' | b'\x0c' | b'\n' | b'\r' => self.i += 1,
                b'#' => self.comment(),
                b'\\' => self.backslash(),
                b'\'' | b'"' => self.string(),
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.token(1);
                }
                b')' | b']' => {
                    depth = depth.saturating_sub(1);
                    self.token(1);
                }
                b'}' => {
                    self.token(1);
                    if depth == 0 && fragment.is_some() {
                        return false;
                    }
                    depth = depth.saturating_sub(1);
                }
                b'!' if depth == 0 && fragment.is_some() && self.at(1) != Some(b'=') => {
                    // A conversion (`!r`), up to the format or the end.
                    let start = self.i;
                    self.i += 1;
                    while let Some(c) = self.at(0) {
                        if matches!(c, b'=' | b':' | b'\'' | b'"' | b'}' | b' ' | b'\t')
                            || c == b'\r'
                            || c == b'\n'
                        {
                            break;
                        }
                        self.i += 1;
                    }
                    self.mark(start, self.i);
                }
                b':' if depth == 0 => match fragment {
                    // The format spec ends the fragment.
                    Some(fragment) => {
                        self.token(1);
                        return self.fstring_text(fragment, true);
                    }
                    None => self.token(1),
                },
                b'0'..=b'9' => {
                    let start = self.i;
                    while self
                        .at(0)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
                    {
                        self.i += 1;
                    }
                    self.mark(start, self.i);
                }
                c if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => self.word(),
                _ => self.token(1),
            }
        }
        true
    }

    /// A one-byte or longer code token.
    fn token(&mut self, len: usize) {
        self.mark(self.i, self.i + len);
        self.i += len;
    }

    fn comment(&mut self) {
        while self.at(0).is_some_and(|c| c != b'\n' && c != b'\r') {
            self.i += 1;
        }
    }

    /// A backslash before the end of the line joins lines and is
    /// whitespace; elsewhere it is a token of its own.
    fn backslash(&mut self) {
        let mut j = self.i + 1;
        while matches!(self.s.get(j), Some(b' ' | b'\t' | b'\x0c')) {
            j += 1;
        }
        if matches!(self.s.get(j), Some(b'\n' | b'\r')) {
            self.i = j;
        } else {
            self.token(1);
        }
    }

    /// A name, or the prefix of a string literal (`rb'...'`) or an
    /// f-string (`f"..."`, `rf'''...'''`, `t"..."`).
    fn word(&mut self) {
        let start = self.i;
        while self
            .at(0)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80)
        {
            self.i += 1;
        }
        if matches!(self.at(0), Some(b'\'' | b'"')) {
            let prefix = &self.s[start..self.i];
            if is_string_prefix(prefix) {
                self.string();
                return;
            }
            if is_fstring_prefix(prefix) {
                self.fstring(start);
                return;
            }
        }
        self.mark(start, self.i);
    }

    /// A string literal (after its prefix, if any) from its quote at the
    /// cursor: not code.
    fn string(&mut self) {
        let q = self.s[self.i];
        if self.at(1) == Some(q) && self.at(2) == Some(q) {
            self.i += 3;
            while let Some(c) = self.at(0) {
                if c == b'\\' {
                    self.i += 2;
                } else if c == q && self.at(1) == Some(q) && self.at(2) == Some(q) {
                    self.i += 3;
                    return;
                } else {
                    self.i += 1;
                }
            }
        } else {
            self.i += 1;
            while let Some(c) = self.at(0) {
                match c {
                    b'\\' => {
                        self.i += if self.at(1) == Some(b'\r') && self.at(2) == Some(b'\n') {
                            3
                        } else {
                            2
                        };
                    }
                    b'\n' | b'\r' => return,
                    c if c == q => {
                        self.i += 1;
                        return;
                    }
                    _ => self.i += 1,
                }
            }
        }
        self.i = self.i.min(self.s.len());
    }

    /// An f-string from its prefix at `start`: its start, text, fragments
    /// and end are code; string literals inside its fragments are not.
    fn fstring(&mut self, start: usize) {
        let q = self.s[self.i];
        let triple = self.at(1) == Some(q) && self.at(2) == Some(q);
        self.i += if triple { 3 } else { 1 };
        self.mark(start, self.i);
        self.fstring_text(Fragment { quote: q, triple }, false);
    }

    /// The literal text of an f-string, or of a fragment's format spec
    /// after its colon (`format`): up to the f-string's end (true) or, in a
    /// format spec, the fragment's closing brace (false).
    fn fstring_text(&mut self, fragment: Fragment, format: bool) -> bool {
        let q = fragment.quote;
        while let Some(c) = self.at(0) {
            match c {
                b'\\' => {
                    let start = self.i;
                    self.i += 2;
                    // `\N{NAME}` is one escape.
                    if self.s.get(start + 1) == Some(&b'N') && self.at(0) == Some(b'{') {
                        while self
                            .at(0)
                            .is_some_and(|c| c != b'}' && c != b'\n' && c != b'\'' && c != b'"')
                        {
                            self.i += 1;
                        }
                        if self.at(0) == Some(b'}') {
                            self.i += 1;
                        }
                    }
                    self.mark(start, self.i);
                }
                b'{' if !format && self.at(1) == Some(b'{') => self.token(2),
                b'}' if !format && self.at(1) == Some(b'}') => self.token(2),
                b'{' => {
                    self.token(1);
                    if self.code(Some(fragment)) {
                        return true;
                    }
                }
                b'}' if format => {
                    self.token(1);
                    return false;
                }
                b'\n' | b'\r' if !fragment.triple => return true,
                c if c == q => {
                    if !fragment.triple {
                        self.token(1);
                        return true;
                    }
                    if self.at(1) == Some(q) && self.at(2) == Some(q) {
                        self.token(3);
                        return true;
                    }
                    self.token(1);
                }
                _ => self.token(1),
            }
        }
        true
    }
}

/// `[UuBbCcRr]{0,3}`, the reference lexer's string prefix.
fn is_string_prefix(prefix: &[u8]) -> bool {
    prefix.len() <= 3 && prefix.iter().all(|c| b"UuBbCcRr".contains(c))
}

/// `[UuBbCcRr]{0,3}[fFtT][UuBbCcRr]{0,3}`, the reference lexer's f-string
/// (and t-string) prefix.
fn is_fstring_prefix(prefix: &[u8]) -> bool {
    let Some(at) = prefix.iter().position(|c| b"fFtT".contains(c)) else {
        return false;
    };
    is_string_prefix(&prefix[..at]) && is_string_prefix(&prefix[at + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines with `# +` (in a comment or in a string) are stoppable,
    /// the others are not.
    fn check(text: &str) {
        let lines = StoppableLines::of(text);
        for (i, line) in text.split('\n').enumerate() {
            let n = i as u32 + 1;
            let expected = line.contains("# +");
            assert_eq!(
                lines.contains(n),
                expected,
                "line {n}: {line:?} should {}be stoppable",
                if expected { "" } else { "not " }
            );
        }
        assert_eq!(lines.line_count(), text.split('\n').count());
    }

    #[test]
    fn a_sample_file() {
        check(concat!(
            "\"\"\"The module docstring\n",
            "over two lines.\"\"\"\n",
            "import os  # +\n",
            "\n",
            "# a comment\n",
            "   \t\n",
            "\n",
            "@decorator  # +\n",
            "def describe(items: list[int]) -> dict[str, int]:  # +\n",
            "    '''A docstring.'''\n",
            "    summary = {  # +\n",
            "        \"count\": len(items),  # +\n",
            "        \"sum\": 0,  # +\n",
            "    }  # +\n",
            "    for value in items:  # +\n",
            "        summary[\"sum\"] += value  # +\n",
            "    else:  # +\n",
            "        pass  # +\n",
            "    'a bare string'\n",
            "    \"a bare string\" # with a comment\n",
            "    \"a string\".upper()  # +\n",
            "    return summary  # +\n",
        ));
    }

    #[test]
    fn continuation_lines_are_not_stoppable() {
        let text = concat!(
            "total = 1 + \\\n",
            "    2\n",
            "x = 3\n",
            "# a comment ending with a backslash \\\n",
            "y = 4\n",
            "z = 5 \\  \t\n",
            "    + 6\n",
            "\\\n",
            "w = 6\n",
        );
        let lines = StoppableLines::of(text);
        // Lines 2, 5, 7 and 9 follow a backslash (the reference looks at
        // the text, a comment's backslash included).
        assert_eq!(lines.lines().collect::<Vec<_>>(), vec![1, 3, 6]);
    }

    #[test]
    fn strings_over_several_lines() {
        check(concat!(
            "text = \"\"\"first  # +\n",
            "inside the string\n",
            "\n",
            "still inside\"\"\" + suffix  # +\n",
            "after = '''a # +\n",
            "b'''\n",
            "single = 'a # + \\\n",
            "b'\n",
            "c = 'unterminated  # +\n",
            "d = 1  # +\n",
            "e = r\"\\\"\"  # +\n",
            "f = rb'''x # +\n",
            "''' # closes\n",
            "g = BR\"x\"  # +\n",
        ));
    }

    #[test]
    fn f_strings_are_not_string_literals() {
        check(concat!(
            "f\"a bare f-string\"  # +\n",
            "t'a bare t-string'  # +\n",
            "message = f\"\"\"Hello  # +\n",
            "{name} and {{braces}}  # +\n",
            "\n",
            "   text  # +\n",
            "{ 'a string in a fragment' }\"\"\"  # +\n",
            "after = rf'{value!r:>{width}}' 'and' f'{x}'  # +\n",
            "nested = f\"{f'{1}'}\"  # +\n",
            "z = 1  # +\n",
        ));
    }

    #[test]
    fn prefixes_are_only_the_lexers() {
        // `xr` is a name followed by a string; `fx` is a name too.
        let text = "xr'a'\nfx'b'\nur'c'\n";
        let lines = StoppableLines::of(text);
        assert!(lines.contains(1));
        assert!(lines.contains(2));
        assert!(!lines.contains(3));
        assert!(is_fstring_prefix(b"rf"));
        assert!(is_fstring_prefix(b"Fr"));
        assert!(!is_fstring_prefix(b"ff"));
        assert!(!is_string_prefix(b"rbur"));
    }

    #[test]
    fn line_numbers_and_endings() {
        let text = "x = 1\r\n\r\n# c\r\ny = 2";
        let lines = StoppableLines::of(text);
        assert_eq!(lines.lines().collect::<Vec<_>>(), vec![1, 4]);
        assert!(!lines.contains(0));
        assert!(!lines.contains(5));
        assert!(is_stoppable(text, 4));
        assert_eq!(StoppableLines::of("").line_count(), 1);
        assert!(!is_stoppable("", 1));
        // A vertical tab is a bad character, a token of its own.
        assert!(is_stoppable("\x0b", 1));
        // Names are not ASCII only.
        assert!(is_stoppable("  größe = 1", 1));
    }
}
