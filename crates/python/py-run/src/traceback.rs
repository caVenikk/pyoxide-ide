use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use console::{Link, LinkFilter, LinkTarget};
use regex::Regex;

/// The reference's Python traceback filter (`PythonTracebackFilter` with the
/// `PyTestTracebackParser` and `PyTracebackParser` parsers): links
/// `File "<path>", line <n>` (the path between the quotes) and pytest's
/// `<path>:<n>` to the file at the line when the file exists. The path is
/// tried as written (`~` is the home folder), then against the working
/// directory. A file whose extension is not `py` gets no link, and
/// `File "<string>"` and similar never exist. At most one link per line.
#[derive(Debug, Clone, Default)]
pub struct TracebackFilter {
    working_dir: Option<PathBuf>,
}

impl TracebackFilter {
    /// A filter resolving relative paths against `working_dir` (the run's
    /// working directory).
    pub fn new(working_dir: Option<PathBuf>) -> Self {
        TracebackFilter { working_dir }
    }

    /// `PythonTracebackFilter.findFileByName`.
    fn find_file(&self, name: &str) -> Option<PathBuf> {
        let prepared = match name.strip_prefix('~') {
            Some(rest) => {
                let home = std::env::home_dir()?;
                format!("{}{rest}", home.to_string_lossy())
            }
            None => name.to_owned(),
        };
        let path = Path::new(&prepared);
        if path.is_absolute() && path.exists() {
            return Some(path.to_path_buf());
        }
        let dir = self.working_dir.as_ref()?;
        let joined = dir.join(&prepared);
        joined.exists().then_some(joined)
    }
}

impl LinkFilter for TracebackFilter {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        for parser in [pytest_link, traceback_link] {
            let Some(found) = parser(line) else {
                continue;
            };
            let Some(path) = self.find_file(&found.file) else {
                continue;
            };
            if !path.is_dir() && path.extension().is_some_and(|e| e != "py") {
                return;
            }
            if found.start < found.end
                && found.end <= line.len()
                && line.is_char_boundary(found.start)
                && line.is_char_boundary(found.end)
            {
                out.push(Link {
                    range: found.start..found.end,
                    target: LinkTarget::File {
                        path,
                        line: u32::try_from(found.line).ok().filter(|n| *n > 0),
                        column: None,
                    },
                });
            }
            return;
        }
    }
}

/// A link found by a parser (`LinkInTrace`): file name as written (with `/`
/// for `\`), line number, byte range in the line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    file: String,
    line: i64,
    start: usize,
    end: usize,
}

/// `TraceBackParserAdapter.MAX_LINE_TO_PARSE`, in characters.
const MAX_LINE_TO_PARSE: usize = 5000;

/// `PyTracebackParser`'s pattern (`\d` is ASCII in Java).
static TRACEBACK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(File "(?<file>[^0-9][^"]{0,200})", line (?<line>[0-9]{1,8}))|(File (?<file2>[^0-9][^"]{0,200}):(?<line2>[0-9]{1,8}))"#,
    )
    .expect("traceback pattern")
});

/// `PyTracebackParser.findLinkInTrace`. In the quoted form the link covers
/// what lies between the line's first two quotes.
fn traceback_link(line: &str) -> Option<Found> {
    let line = match line.char_indices().nth(MAX_LINE_TO_PARSE) {
        Some((cut, _)) => &line[..cut],
        None => line,
    };
    let caps = TRACEBACK.captures(line)?;
    if let (Some(file), Some(number)) = (caps.name("file"), caps.name("line")) {
        let start = line.find('"')? + 1;
        let end = start + line[start..].find('"')?;
        return Some(Found {
            file: file.as_str().replace('\\', "/"),
            line: number.as_str().parse().ok()?,
            start,
            end,
        });
    }
    let file = caps.name("file2")?;
    let number = caps.name("line2")?;
    Some(Found {
        file: file.as_str().replace('\\', "/"),
        line: number.as_str().parse().ok()?,
        start: file.start(),
        end: number.end(),
    })
}

/// `PyTestTracebackParser`: `path:line` either inside double quotes or
/// between spaces, found by two `PyFilesStateMachine`s. The line is treated
/// as ending with its line break, as the reference sees complete lines.
fn pytest_link(line: &str) -> Option<Found> {
    let mut quote = Machine::new(true);
    let mut space = Machine::new(false);
    let end = line.len();
    for (i, c) in line.char_indices().chain(std::iter::once((end, '\n'))) {
        if quote.add(c, i) {
            return quote.found();
        }
        if space.add(c, i) {
            return space.found();
        }
    }
    if space.end_line() {
        return space.found();
    }
    None
}

/// `PyFilesStateMachine`: looks for a file name and a line number, in
/// "quote mode" (inside double quotes, spaces allowed) or "space mode"
/// (between spaces or the line's ends).
struct Machine {
    quote_mode: bool,
    looking_for_file: bool,
    in_progress: bool,
    after_colon: bool,
    start: usize,
    file: String,
    line: String,
}

impl Machine {
    fn new(quote_mode: bool) -> Self {
        Machine {
            quote_mode,
            looking_for_file: true,
            in_progress: !quote_mode,
            after_colon: false,
            start: 0,
            file: String::new(),
            line: String::new(),
        }
    }

    /// Feeds the character at byte `pos`; `true` when a link was found.
    fn add(&mut self, c: char, pos: usize) -> bool {
        if !self.in_progress {
            if c == '"' && self.quote_mode {
                self.in_progress = true;
                self.start = pos;
            }
            return false;
        }
        if self.after_colon {
            if c == '/' || c == '\\' {
                self.file.push(':');
                self.file.push(c);
                self.after_colon = false;
                return false;
            }
            if c.is_ascii_digit() {
                self.looking_for_file = false;
                self.line.push(c);
                self.after_colon = false;
                return false;
            }
            self.reset();
            return false;
        }
        if c == '"' {
            if self.quote_mode && !self.file.is_empty() && !self.line.is_empty() {
                return true;
            }
            self.reset();
            if self.quote_mode {
                self.in_progress = true;
                self.start = pos;
            }
            return false;
        }
        if c == ' ' {
            if self.quote_mode && self.looking_for_file {
                self.file.push(' ');
                return false;
            }
            if !self.quote_mode && !self.file.is_empty() && !self.line.is_empty() {
                return true;
            }
            self.reset();
            if !self.quote_mode {
                self.in_progress = true;
                self.start = pos + 1;
            }
            return false;
        }
        if (c.is_alphabetic() || matches!(c, '/' | '\\' | '.' | '_' | '-')) && self.looking_for_file
        {
            self.file.push(c);
            return false;
        }
        if c == ':' && self.looking_for_file {
            self.after_colon = true;
            return false;
        }
        if c.is_ascii_digit() {
            if !self.looking_for_file {
                self.line.push(c);
                return false;
            }
            if self.file.is_empty() {
                self.reset();
                return false;
            }
            self.file.push(c);
            return false;
        }
        if !self.file.is_empty() && !self.line.is_empty() {
            return true;
        }
        self.reset();
        false
    }

    fn reset(&mut self) {
        self.in_progress = false;
        self.looking_for_file = true;
        self.line.clear();
        self.file.clear();
    }

    fn end_line(&self) -> bool {
        !self.quote_mode && !self.file.is_empty() && !self.line.is_empty()
    }

    /// `PyTestTracebackParser.createLinkInTrace`: the line number is read
    /// from its first five digits; the range is `start` plus the file, the
    /// colon and the number (in quote mode it starts at the quote).
    fn found(&self) -> Option<Found> {
        let digits = if self.line.len() > 5 {
            &self.line[..5]
        } else {
            &self.line
        };
        Some(Found {
            file: self.file.clone(),
            line: digits.parse().ok()?,
            start: self.start,
            end: self.start + self.file.len() + self.line.len() + 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(file: &str, line: i64, start: usize, end: usize) -> Found {
        Found {
            file: file.to_owned(),
            line,
            start,
            end,
        }
    }

    #[test]
    fn cpython_traceback_lines() {
        let line = r#"  File "/home/u/p/main.py", line 12, in <module>"#;
        assert_eq!(pytest_link(line), None);
        let f = traceback_link(line).unwrap();
        assert_eq!(f, found("/home/u/p/main.py", 12, 8, 25));
        assert_eq!(&line[f.start..f.end], "/home/u/p/main.py");
        // Relative paths and module frames.
        let line = r#"  File "pkg/mod.py", line 3, in f"#;
        assert_eq!(traceback_link(line).unwrap().file, "pkg/mod.py");
        let line = r#"  File "<string>", line 1, in <module>"#;
        assert_eq!(traceback_link(line).unwrap().file, "<string>");
        // Windows paths: backslashes become slashes in the file name only.
        let line = r#"  File "C:\Users\u\p\main.py", line 5, in <module>"#;
        assert_eq!(pytest_link(line), None);
        let f = traceback_link(line).unwrap();
        assert_eq!(f.file, "C:/Users/u/p/main.py");
        assert_eq!(&line[f.start..f.end], r"C:\Users\u\p\main.py");
        // Code and marker lines have no links.
        for line in [
            "Traceback (most recent call last):",
            "    main()",
            "    ~~~~^^",
            "    1/0",
            "    ~^~",
            "ZeroDivisionError: division by zero",
        ] {
            assert_eq!(pytest_link(line), None, "{line}");
            assert_eq!(traceback_link(line), None, "{line}");
        }
    }

    #[test]
    fn file_colon_line_form() {
        let line = "File /home/u/p/main.py:7 something";
        let f = traceback_link(line).unwrap();
        assert_eq!(f.file, "/home/u/p/main.py");
        assert_eq!(&line[f.start..f.end], "/home/u/p/main.py:7");
    }

    #[test]
    fn pytest_lines() {
        let line = "tests/test_a.py:12: AssertionError";
        let f = pytest_link(line).unwrap();
        assert_eq!(f, found("tests/test_a.py", 12, 0, 18));
        assert_eq!(&line[f.start..f.end], "tests/test_a.py:12");
        let line = "E   at /tmp/x_y-z.py:3";
        let f = pytest_link(line).unwrap();
        assert_eq!(&line[f.start..f.end], "/tmp/x_y-z.py:3");
        let line = r"see C:\work\a.py:40 here";
        let f = pytest_link(line).unwrap();
        assert_eq!(f.file, r"C:\work\a.py");
        assert_eq!(f.line, 40);
        // In quote mode the range starts at the quote and stops one short,
        // as in the reference.
        let line = r#"x "a b.py:12" y"#;
        let f = pytest_link(line).unwrap();
        assert_eq!(f.file, "a b.py");
        assert_eq!(&line[f.start..f.end], "\"a b.py:1");
        // Only the first five digits of a long number count.
        assert_eq!(pytest_link("a.py:1234567").unwrap().line, 12345);
        assert_eq!(pytest_link("12:30 started"), None);
    }

    #[test]
    fn links_need_an_existing_python_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pkg")).unwrap();
        std::fs::write(dir.path().join("pkg").join("mod.py"), "").unwrap();
        std::fs::write(dir.path().join("data.txt"), "").unwrap();
        let main = dir.path().join("main.py");
        std::fs::write(&main, "").unwrap();
        let filter = TracebackFilter::new(Some(dir.path().to_path_buf()));
        let links = |line: &str| {
            let mut out = Vec::new();
            filter.links(line, &mut out);
            out
        };

        let line = format!("  File \"{}\", line 12, in <module>", main.display());
        let found = links(&line);
        assert_eq!(found.len(), 1);
        assert_eq!(&line[found[0].range.clone()], main.display().to_string());
        assert_eq!(
            found[0].target,
            LinkTarget::File {
                path: main.clone(),
                line: Some(12),
                column: None
            }
        );
        // Relative to the working directory.
        let found = links(r#"  File "pkg/mod.py", line 3, in f"#);
        assert_eq!(
            found[0].target,
            LinkTarget::File {
                path: dir.path().join("pkg/mod.py"),
                line: Some(3),
                column: None
            }
        );
        // Not a Python file, missing files and pseudo files: no link.
        assert!(links(r#"  File "data.txt", line 1, in <module>"#).is_empty());
        assert!(links(r#"  File "missing.py", line 1, in <module>"#).is_empty());
        assert!(links(r#"  File "<string>", line 1, in <module>"#).is_empty());
        assert!(links(r#"  File "<frozen runpy>", line 88, in _run_code"#).is_empty());
        // pytest form.
        let found = links("pkg/mod.py:7: in test_x");
        assert_eq!(found[0].range, 0..12);
        // Without a working directory only absolute paths resolve.
        let filter = TracebackFilter::new(None);
        let mut out = Vec::new();
        filter.links(r#"  File "pkg/mod.py", line 3, in f"#, &mut out);
        assert!(out.is_empty());
    }
}
