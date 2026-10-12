//! The lexical layer of Python highlighting: colours one line at a time
//! without the parse tree, so code keeps its colours while it is
//! syntactically incomplete. Its roles follow the highlight query
//! (`queries/highlights.scm`) wherever a token alone decides them: keywords,
//! literals, comments, decorators, operators, built-ins, the name after
//! `def` or `class`, `self`, dunder names and calls. The tree refines the
//! rest (methods, docstrings, keyword arguments, f-string expressions).
//!
//! The state carried between lines is the open triple-quoted string, if
//! any: bits 0-1 the quote (1 `'''`, 2 `"""`), bit 2 a raw string.

use std::ops::Range;

use syntax::{LexSpan, Lexer};
use themes::TextRole;

/// Hard keywords (soft keywords such as `match` are left to the tree).
const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

const CONSTANTS: &[&str] = &["True", "False", "None"];

/// Built-in functions, types and exceptions (the list of the query).
const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "EOFError",
    "ImportError",
    "IndexError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "NameError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "RecursionError",
    "RuntimeError",
    "StopIteration",
    "StopAsyncIteration",
    "SyntaxError",
    "SystemExit",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
    "FileNotFoundError",
    "PermissionError",
    "TimeoutError",
    "NotImplemented",
];

const SINGLE: u32 = 1;
const DOUBLE: u32 = 2;
const RAW: u32 = 4;

/// The Python line lexer.
#[derive(Debug, Default, Clone, Copy)]
pub struct PythonLexer;

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_dunder(word: &str) -> bool {
    word.len() > 4
        && word.starts_with("__")
        && word.ends_with("__")
        && word[2..word.len() - 2]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A string prefix: up to two of r, b, f, u, t (any case).
fn is_string_prefix(word: &str) -> bool {
    word.len() <= 2 && word.chars().all(|c| "rRbBfFuUtT".contains(c))
}

/// Comments with these markers are drawn as TODO items (as in the query).
fn is_todo(comment: &str) -> bool {
    ["TODO", "FIXME", "todo", "fixme"]
        .iter()
        .any(|m| comment.contains(m))
}

/// Byte length of a valid escape sequence at the start of `s` (which starts
/// with a backslash), or 0.
fn escape_len(s: &str) -> usize {
    let b = s.as_bytes();
    if b.len() < 2 {
        return 0;
    }
    match b[1] {
        b'\\' | b'\'' | b'"' | b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => 2,
        b'0'..=b'7' => {
            1 + b[1..]
                .iter()
                .take(3)
                .take_while(|c| (b'0'..=b'7').contains(c))
                .count()
        }
        b'x' => hex_escape(b, 2),
        b'u' => hex_escape(b, 4),
        b'U' => hex_escape(b, 8),
        b'N' if b.get(2) == Some(&b'{') => s.find('}').map_or(0, |e| e + 1),
        _ => 0,
    }
}

fn hex_escape(b: &[u8], digits: usize) -> usize {
    let ok = b.len() >= 2 + digits && b[2..2 + digits].iter().all(u8::is_ascii_hexdigit);
    if ok { 2 + digits } else { 0 }
}

/// Lexes the body of a string from `i` to its closing quote (or the end of
/// the line), adding escape spans; returns the end of the string and
/// whether it was closed.
fn string_body(
    line: &str,
    mut i: usize,
    quote: &str,
    raw: bool,
    out: &mut Vec<LexSpan>,
) -> (usize, bool) {
    let bytes = line.as_bytes();
    while i < line.len() {
        if bytes[i] == b'\\' {
            let n = escape_len(&line[i..]);
            if n > 0 && !raw {
                out.push((i..i + n, TextRole::StringEscape));
                i += n;
            } else {
                // An escaped character never closes the string.
                i += line[i..].chars().take(2).map(char::len_utf8).sum::<usize>();
            }
            continue;
        }
        if line[i..].starts_with(quote) {
            return (i + quote.len(), true);
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    (line.len(), false)
}

/// Adds a string span from `start` to `end` under its escape spans.
fn push_string(out: &mut Vec<LexSpan>, from: usize, range: Range<usize>) {
    // Escape spans were pushed first; the string goes before them so that
    // the escapes paint over it.
    out.insert(from, (range, TextRole::String));
}

impl Lexer for PythonLexer {
    fn lex_line(&self, line: &str, state: u32, out: &mut Vec<LexSpan>) -> u32 {
        let bytes = line.as_bytes();
        let mut i = 0;
        // Inside a triple-quoted string from an earlier line.
        if state != 0 {
            let quote = if state & 3 == SINGLE { "'''" } else { "\"\"\"" };
            let from = out.len();
            let (end, closed) = string_body(line, 0, quote, state & RAW != 0, out);
            push_string(out, from, 0..end);
            if !closed {
                return state;
            }
            i = end;
        }
        // The keyword before the current word (`def`, `class`).
        let mut previous_keyword: Option<&str> = None;
        let mut first_token = true;
        while i < line.len() {
            let c = line[i..].chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                i += c.len_utf8();
                continue;
            }
            let at_line_start = first_token;
            first_token = false;
            if c == '#' {
                let role = if is_todo(&line[i..]) {
                    TextRole::Todo
                } else {
                    TextRole::Comment
                };
                out.push((i..line.len(), role));
                return 0;
            }
            // A string, possibly with a prefix.
            let word_end = i + line[i..]
                .char_indices()
                .find(|&(_, ch)| !is_ident(ch))
                .map_or(line.len() - i, |(n, _)| n);
            let prefix = &line[i..word_end];
            let quote_at = if c == '\'' || c == '"' {
                Some(i)
            } else if is_ident_start(c)
                && is_string_prefix(prefix)
                && matches!(bytes.get(word_end), Some(b'\'' | b'"'))
            {
                Some(word_end)
            } else {
                None
            };
            if let Some(q) = quote_at {
                let raw = line[i..q].contains(['r', 'R']);
                let ch = bytes[q] as char;
                let triple: String = std::iter::repeat_n(ch, 3).collect();
                let from = out.len();
                if line[q..].starts_with(&triple) {
                    let (end, closed) = string_body(line, q + 3, &triple, raw, out);
                    push_string(out, from, i..end);
                    if !closed {
                        let kind = if ch == '\'' { SINGLE } else { DOUBLE };
                        return kind | if raw { RAW } else { 0 };
                    }
                    i = end;
                } else {
                    let quote = ch.to_string();
                    let (end, _) = string_body(line, q + 1, &quote, raw, out);
                    push_string(out, from, i..end);
                    i = end;
                }
                previous_keyword = None;
                continue;
            }
            // Numbers.
            if c.is_ascii_digit() || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                let hex = line[i..].starts_with("0x") || line[i..].starts_with("0X");
                let mut end = i + 1;
                while end < line.len() {
                    let b = bytes[end];
                    let exponent_sign =
                        !hex && matches!(b, b'+' | b'-') && matches!(bytes[end - 1], b'e' | b'E');
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || exponent_sign {
                        end += 1;
                    } else {
                        break;
                    }
                }
                out.push((i..end, TextRole::Number));
                i = end;
                previous_keyword = None;
                continue;
            }
            // Names.
            if is_ident_start(c) {
                let word = prefix;
                let after = line[word_end..].trim_start();
                let called = after.starts_with('(');
                let attribute =
                    line[..i].trim_end().ends_with('.') && !line[..i].trim_end().ends_with("..");
                let role = if KEYWORDS.contains(&word) {
                    Some(TextRole::Keyword)
                } else if CONSTANTS.contains(&word) {
                    Some(TextRole::ConstantBuiltin)
                } else if previous_keyword == Some("def") {
                    Some(if is_dunder(word) {
                        TextRole::Dunder
                    } else {
                        TextRole::FunctionDecl
                    })
                } else if previous_keyword == Some("class") {
                    Some(TextRole::ClassDecl)
                } else if word == "self" || word == "cls" {
                    Some(TextRole::SelfParameter)
                } else if is_dunder(word) {
                    Some(TextRole::Dunder)
                } else if attribute {
                    called.then_some(TextRole::MethodCall)
                } else if BUILTINS.contains(&word) {
                    Some(TextRole::Builtin)
                } else if called {
                    Some(TextRole::FunctionCall)
                } else {
                    None
                };
                if let Some(role) = role {
                    out.push((i..word_end, role));
                }
                previous_keyword = KEYWORDS.contains(&word).then_some(word);
                i = word_end;
                continue;
            }
            previous_keyword = None;
            // A decorator: `@` and a dotted name at the start of a line.
            if c == '@' && at_line_start {
                let mut end = i + 1;
                while end < line.len() && (is_ident(bytes[end] as char) || bytes[end] == b'.') {
                    end += 1;
                }
                out.push((i..end, TextRole::Decorator));
                i = end;
                continue;
            }
            if line[i..].starts_with("...") {
                out.push((i..i + 3, TextRole::ConstantBuiltin));
                i += 3;
                continue;
            }
            if line[i..].starts_with(":=") || line[i..].starts_with("->") {
                out.push((i..i + 2, TextRole::Operator));
                i += 2;
                continue;
            }
            if "+-*/%=<>!&|^~@".contains(c) {
                let mut end = i + 1;
                while end < line.len() && "=<>*/".contains(bytes[end] as char) && end - i < 3 {
                    end += 1;
                }
                out.push((i..end, TextRole::Operator));
                i = end;
                continue;
            }
            i += c.len_utf8();
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextRole::*;

    fn lex(lines: &[&str]) -> Vec<Vec<(std::string::String, TextRole)>> {
        let mut state = 0;
        lines
            .iter()
            .map(|line| {
                let mut spans = Vec::new();
                state = PythonLexer.lex_line(line, state, &mut spans);
                spans
                    .into_iter()
                    .map(|(r, role)| (line[r].to_owned(), role))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn tokens_of_one_line() {
        let out = lex(&["def fib(n: int) -> int:  # TODO memo"]);
        assert_eq!(
            out[0],
            [
                ("def".to_owned(), Keyword),
                ("fib".to_owned(), FunctionDecl),
                ("int".to_owned(), Builtin),
                ("->".to_owned(), Operator),
                ("int".to_owned(), Builtin),
                ("# TODO memo".to_owned(), Todo),
            ]
        );
        let out = lex(&["x = f'a{b}\\n' + rb\"\\d\" + 0x1F + 1_000.5e-3j"]);
        assert_eq!(
            out[0],
            [
                ("=".to_owned(), Operator),
                ("f'a{b}\\n'".to_owned(), String),
                ("\\n".to_owned(), StringEscape),
                ("+".to_owned(), Operator),
                ("rb\"\\d\"".to_owned(), String),
                ("+".to_owned(), Operator),
                ("0x1F".to_owned(), Number),
                ("+".to_owned(), Operator),
                ("1_000.5e-3j".to_owned(), Number),
            ]
        );
        let out = lex(&[
            "@app.route('/')",
            "obj.method(self, None, ...)",
            "class Foo(Base):",
        ]);
        assert_eq!(out[0][0], ("@app.route".to_owned(), Decorator));
        assert_eq!(out[1][0], ("method".to_owned(), MethodCall));
        assert_eq!(out[1][1], ("self".to_owned(), SelfParameter));
        assert_eq!(out[1][2], ("None".to_owned(), ConstantBuiltin));
        assert_eq!(out[1][3], ("...".to_owned(), ConstantBuiltin));
        assert_eq!(out[2][1], ("Foo".to_owned(), ClassDecl));
    }

    #[test]
    fn triple_quoted_strings_span_lines() {
        let out = lex(&[
            "s = \"\"\"start",
            "middle 'x' # not a comment",
            "end\"\"\" + 1",
        ]);
        assert_eq!(out[0][1], ("\"\"\"start".to_owned(), String));
        assert_eq!(out[1], [("middle 'x' # not a comment".to_owned(), String)]);
        assert_eq!(out[2][0], ("end\"\"\"".to_owned(), String));
        assert_eq!(out[2][2], ("1".to_owned(), Number));
    }

    #[test]
    fn unterminated_strings_end_with_the_line() {
        let out = lex(&["s = 'abc", "def f(): pass"]);
        assert_eq!(out[0][1], ("'abc".to_owned(), String));
        assert_eq!(out[1][0], ("def".to_owned(), Keyword));
        assert_eq!(out[1][1], ("f".to_owned(), FunctionDecl));
    }
}
