//! What the variables view needs from Python about the expressions of the
//! evaluate field and the watches, and about their values:
//!
//! - a failed evaluation as the reference's Python rows show it,
//!   `{NameError}NameError("name 'x' is not defined")` (pydevd's error value:
//!   the exception's class and its `repr`), from debugpy's message
//!   `NameError: name 'x' is not defined`;
//! - View as for integers (`PyNodeRendererManager`: Binary, Decimal, Hex;
//!   `BinaryRenderer`, `HexRenderer` with `BigInteger`);
//! - the names of an expression the frame's code does not declare (the
//!   reference's code fragment marks unresolved references; here a name
//!   is declared when it is a builtin, or a word of code in the scope of
//!   the frame's function or on the module level, see [`crate::declaration`]);
//! - where the type of a value is declared (Jump To Type Source), asked of
//!   the running program with `inspect`.

use std::ops::Range;
use std::path::PathBuf;

use crate::declaration::declaration;

/// The renderers of an integer, in the order View as lists them, and the
/// default one.
pub(crate) const INTEGER_RENDERERS: [&str; 3] = ["Binary", "Decimal", "Hex"];
pub(crate) const DEFAULT_RENDERER: &str = "Decimal";

/// The type and the text of a failed evaluation from debugpy's message
/// (its last line: `NameError: name 'x' is not defined`, `KeyError: 'k'`,
/// `StopIteration`); `None` when the message names no exception.
pub(crate) fn evaluation_error(message: &str) -> Option<(String, String)> {
    let line = message
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    let (class, text) = match line.split_once(':') {
        Some((class, text)) => (class.trim(), Some(text.trim())),
        None => (line, None),
    };
    let valid = !class.is_empty()
        && class.split('.').all(is_identifier)
        && class
            .rsplit('.')
            .next()
            .is_some_and(|c| c.starts_with(|ch: char| ch.is_uppercase()));
    if !valid {
        return None;
    }
    let class = class.rsplit('.').next().unwrap_or(class).to_owned();
    let args = match text {
        Some(text) if !text.is_empty() => python_repr(text),
        _ => String::new(),
    };
    let text = format!("{class}({args})");
    Some((class, text))
}

/// A string as Python's `repr` writes it: single quotes unless the text has
/// one and no double quote.
pub(crate) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// An integer's text as a renderer shows it: `0b101`, `0x1F`, `-0x1F`;
/// `None` when the value is not an integer or the renderer is unknown.
pub(crate) fn render_integer(renderer: &str, value: &str) -> Option<String> {
    let value = value.trim();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (base, prefix) = match renderer {
        "Decimal" => return Some(value.to_owned()),
        "Binary" => (2, "0b"),
        "Hex" => (16, "0x"),
        _ => return None,
    };
    let mut decimal: Vec<u32> = digits.bytes().map(|b| u32::from(b - b'0')).collect();
    let mut out = Vec::new();
    // Repeated division of the decimal digits by the base.
    loop {
        let mut rest = 0;
        let mut quotient = Vec::with_capacity(decimal.len());
        for d in &decimal {
            let current = rest * 10 + d;
            let q = current / base;
            rest = current % base;
            if !(quotient.is_empty() && q == 0) {
                quotient.push(q);
            }
        }
        out.push(
            char::from_digit(rest, base)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
        if quotient.is_empty() {
            break;
        }
        decimal = quotient;
    }
    let digits: String = out.into_iter().rev().collect();
    Some(format!(
        "{}{prefix}{digits}",
        if negative { "-" } else { "" }
    ))
}

/// Python's keywords and the soft ones an expression may hold.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case", "type", "_",
];

/// The names of the `builtins` module (Python 3.13) and a module's own
/// attributes.
const BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "Ellipsis",
    "EncodingWarning",
    "EnvironmentError",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplemented",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    "__build_class__",
    "__debug__",
    "__import__",
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
    "copyright",
    "credits",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "exit",
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
    "license",
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
    "quit",
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
    "__name__",
    "__file__",
    "__doc__",
    "__package__",
    "__spec__",
    "__loader__",
    "__builtins__",
    "__dict__",
    "__annotations__",
    "__path__",
    "__cached__",
];

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// A token of an expression.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// An identifier and its char range.
    Name(String, Range<usize>),
    /// An operator or a bracket.
    Op(String),
    /// A number or a string.
    Literal,
}

/// The tokens of an expression (strings with their prefixes, numbers,
/// names, operators; comments end the line).
fn tokens(expression: &str) -> Vec<Token> {
    let chars: Vec<char> = expression.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '\\' {
            i += 1;
        } else if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '\'' || c == '"' {
            i = skip_string(&chars, i);
            out.push(Token::Literal);
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                // An exponent's sign belongs to the number.
                if matches!(chars[i], 'e' | 'E') && matches!(chars.get(i + 1), Some('+' | '-')) {
                    i += 1;
                }
                i += 1;
            }
            out.push(Token::Literal);
        } else if c == '_' || c.is_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // A string prefix: `f"…"`, `rb'…'`.
            if i < chars.len()
                && (chars[i] == '\'' || chars[i] == '"')
                && word.len() <= 2
                && word.chars().all(|c| "rRbBfFuUtT".contains(c))
            {
                i = skip_string(&chars, i);
                out.push(Token::Literal);
                continue;
            }
            out.push(Token::Name(word, start..i));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let op = if matches!(
                two.as_str(),
                "==" | "!=" | "<=" | ">=" | ":=" | "->" | "**" | "//" | "<<" | ">>"
            ) {
                two
            } else {
                c.to_string()
            };
            i += op.chars().count();
            out.push(Token::Op(op));
        }
    }
    out
}

/// The index after the string literal that starts at `start` (a quote).
fn skip_string(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let triple = chars.get(start + 1) == Some(&quote) && chars.get(start + 2) == Some(&quote);
    let mut i = start + if triple { 3 } else { 1 };
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            if !triple {
                return i + 1;
            }
            if chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote) {
                return i + 3;
            }
        }
        i += 1;
    }
    chars.len()
}

/// The names an expression uses that it does not bind itself, with their
/// char ranges: not attributes (after a dot), not keyword arguments, not
/// the targets of its comprehensions, lambdas and `:=`.
fn free_names(expression: &str) -> Vec<(String, Range<usize>)> {
    let tokens = tokens(expression);
    let op = |i: usize| match tokens.get(i) {
        Some(Token::Op(o)) => Some(o.as_str()),
        _ => None,
    };
    let mut bound: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Name(word, _) if word == "for" => {
                // `for a, (b, c) in`: the targets up to `in`.
                let mut j = i + 1;
                while j < tokens.len() {
                    match &tokens[j] {
                        Token::Name(w, _) if w == "in" => break,
                        Token::Name(w, _) => bound.push(w.clone()),
                        _ => {}
                    }
                    j += 1;
                }
            }
            Token::Name(word, _) if word == "lambda" => {
                // The parameters up to the colon (defaults are expressions
                // but rarely names; they count as parameters here).
                let mut j = i + 1;
                while j < tokens.len() && op(j) != Some(":") {
                    if let Token::Name(w, _) = &tokens[j] {
                        bound.push(w.clone());
                    }
                    j += 1;
                }
            }
            Token::Name(word, _) if op(i + 1) == Some(":=") => bound.push(word.clone()),
            _ => {}
        }
        i += 1;
    }
    let mut out = Vec::new();
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Op(o) if matches!(o.as_str(), "(" | "[" | "{") => depth += 1,
            Token::Op(o) if matches!(o.as_str(), ")" | "]" | "}") => {
                depth = depth.saturating_sub(1)
            }
            Token::Name(word, range) => {
                let attribute = i > 0 && op(i - 1) == Some(".");
                let keyword_argument = depth > 0 && op(i + 1) == Some("=");
                if attribute
                    || keyword_argument
                    || KEYWORDS.contains(&word.as_str())
                    || bound.contains(word)
                {
                    continue;
                }
                out.push((word.clone(), range.clone()));
            }
            _ => {}
        }
    }
    out
}

/// The names of `expression` that neither the builtins nor the code of the
/// frame suspended on `line` of `text` declare; nothing when the module
/// imports everything of another one (`from m import *`).
pub(crate) fn unresolved_names(text: &str, line: u32, expression: &str) -> Vec<Range<usize>> {
    let star = text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("from ")
            && l.split_whitespace()
                .collect::<Vec<_>>()
                .ends_with(&["import", "*"])
    });
    if star {
        return Vec::new();
    }
    free_names(expression)
        .into_iter()
        .filter(|(name, _)| {
            !BUILTINS.contains(&name.as_str())
                && declaration(text, line, name).is_none()
                && declaration(text, 0, name).is_none()
        })
        .map(|(_, range)| range)
        .collect()
}

/// The expression whose value is `(file, line)` of the declaration of the
/// type of `expression`'s value.
pub(crate) fn type_source_expression(expression: &str) -> String {
    format!(
        "(lambda __t: (__import__('inspect').getsourcefile(__t), \
         __import__('inspect').getsourcelines(__t)[1]))(type(({expression})))"
    )
}

/// `('/path/to/file.py', 12)` as a path and a line.
pub(crate) fn parse_type_source(value: &str) -> Option<(PathBuf, u32)> {
    let inner = value.trim().strip_prefix('(')?.strip_suffix(')')?;
    let (path, line) = inner.rsplit_once(',')?;
    let line: u32 = line.trim().parse().ok()?;
    let path = path.trim();
    let quote = path.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let body = path.strip_prefix(quote)?.strip_suffix(quote)?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) => out.push(n),
                None => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    (!out.is_empty() && line > 0).then(|| (PathBuf::from(out), line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_as_pydevd_shows_them() {
        let e = |m| evaluation_error(m).map(|(t, v)| format!("{{{t}}}{v}"));
        assert_eq!(
            e("NameError: name 'undefined_name' is not defined\n").as_deref(),
            Some("{NameError}NameError(\"name 'undefined_name' is not defined\")")
        );
        assert_eq!(
            e("Traceback (most recent call last):\n  File \"<string>\", line 1\nZeroDivisionError: division by zero")
                .as_deref(),
            Some("{ZeroDivisionError}ZeroDivisionError('division by zero')")
        );
        assert_eq!(
            e("json.decoder.JSONDecodeError: Expecting value").as_deref(),
            Some("{JSONDecodeError}JSONDecodeError('Expecting value')")
        );
        assert_eq!(
            e("StopIteration").as_deref(),
            Some("{StopIteration}StopIteration()")
        );
        assert_eq!(e("Unable to find thread"), None);
        assert_eq!(e("timed out: no answer"), None);
        assert_eq!(e(""), None);
        assert_eq!(python_repr("a'b\"c\n"), "'a\\'b\"c\\n'");
        assert_eq!(python_repr("it's"), "\"it's\"");
    }

    #[test]
    fn integers_in_binary_and_hex() {
        assert_eq!(render_integer("Binary", "5").as_deref(), Some("0b101"));
        assert_eq!(render_integer("Hex", "255").as_deref(), Some("0xFF"));
        assert_eq!(render_integer("Hex", "-31").as_deref(), Some("-0x1F"));
        assert_eq!(render_integer("Hex", "0").as_deref(), Some("0x0"));
        assert_eq!(render_integer("Decimal", "42").as_deref(), Some("42"));
        assert_eq!(
            render_integer("Hex", "340282366920938463463374607431768211456").as_deref(),
            Some("0x100000000000000000000000000000000")
        );
        assert_eq!(render_integer("Hex", "1.5"), None);
        assert_eq!(render_integer("Octal", "8"), None);
    }

    const MAIN: &str = "from demo_pkg.util import total\n\n\ndef describe(items: list[int]) -> dict[str, int]:\n    summary = {\"count\": len(items), \"sum\": 0}\n    for value in items:\n        summary[\"sum\"] += value\n    return summary\n\n\nif __name__ == \"__main__\":\n    numbers = [1, 2, 3, 4]\n    print(describe(numbers))\n";

    fn unresolved(expression: &str) -> Vec<String> {
        let chars: Vec<char> = expression.chars().collect();
        unresolved_names(MAIN, 7, expression)
            .into_iter()
            .map(|r| chars[r].iter().collect())
            .collect()
    }

    #[test]
    fn names_the_frame_does_not_declare() {
        assert_eq!(unresolved("undefined_name"), ["undefined_name"]);
        assert!(unresolved("summary['sum'] + len(items)").is_empty());
        assert!(unresolved("value").is_empty());
        // Module level: imports, functions, names of the main block.
        assert!(unresolved("total(numbers) + describe(items)['sum']").is_empty());
        assert!(unresolved("items.undefined_attribute").is_empty());
        assert!(unresolved("[x * 2 for x in items if x]").is_empty());
        assert!(unresolved("sorted(items, key=lambda v: -v)").is_empty());
        assert!(unresolved("(n := len(items)) + n").is_empty());
        assert!(unresolved("'missing' + f\"{missing}\" # missing").is_empty());
        assert_eq!(unresolved("a + items[b]"), ["a", "b"]);
        assert!(unresolved("None if __name__ else True").is_empty());
        // A star import may bring any name.
        assert!(unresolved_names("from os import *\nx = 1\n", 2, "anything").is_empty());
    }

    #[test]
    fn type_source_from_inspect() {
        assert!(type_source_expression("items").contains("type((items))"));
        assert_eq!(
            parse_type_source("('/work/demo/main.py', 12)"),
            Some((PathBuf::from("/work/demo/main.py"), 12))
        );
        assert_eq!(
            parse_type_source("('C:\\\\work\\\\it\\'s.py', 3)"),
            Some((PathBuf::from("C:\\work\\it's.py"), 3))
        );
        assert_eq!(parse_type_source("None"), None);
        assert_eq!(parse_type_source("('x.py', 0)"), None);
    }
}
