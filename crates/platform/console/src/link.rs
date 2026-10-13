use std::ops::Range;
use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;

/// Where a console hyperlink leads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// A file, optionally at a 1-based line and column. The file may not
    /// exist (a `file:` URL is not checked).
    File {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
    /// A URL to open in the browser, as written in the output (a `www.`
    /// address has no scheme).
    Url(String),
}

/// A hyperlink in a console line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Link {
    /// Byte range of the link text in the line.
    pub range: Range<usize>,
    pub target: LinkTarget,
}

/// Finds hyperlinks in a console line (the reference's console `Filter`).
///
/// [`Console`](crate::Console) calls the filters for every new or changed
/// line, including the current partial line, with the line's text without
/// the line break. A filter may look at the file system (for example to
/// check that a file exists), so the console should be fed off the UI
/// thread.
pub trait LinkFilter: Send + Sync {
    /// Appends the links found in `line` to `out`.
    fn links(&self, line: &str, out: &mut Vec<Link>);
}

/// The reference's URL filter (`UrlFilter` with the `URLUtil` patterns):
/// `http(s)://`, `ftp(s)://`, `news://`, `mailto:` and `www.` links, and
/// `file:` URLs, which become file links with an optional `:line[:column]`
/// suffix (a `.html` file stays a URL).
#[derive(Debug, Clone, Copy, Default)]
pub struct UrlFilter;

/// `URLUtil.URL_PATTERN_OPTIMIZED` without the look-behind before `www.`,
/// which [`UrlFilter`] checks by hand.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u:\b)(?:mailto:|(?:news|(?:ht|f)tps?)://|www\.)[\-A-Za-z0-9+$\&@#/%?=\~_|!:,.;]*[\-A-Za-z0-9+$\&@#/%=\~_|]",
    )
    .expect("URL pattern")
});

/// `URLUtil.FILE_URL_PATTERN_OPTIMIZED` (`\s` is ASCII whitespace in Java).
static FILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u:\b)file:(?:/|[A-Za-z]:)[^ \t\n\x0B\x0C\r()\[\]]*[^ \t\n\x0B\x0C\r()\[\]:;?!,.]",
    )
    .expect("file URL pattern")
});

impl LinkFilter for UrlFilter {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        // URLUtil.canContainUrl
        if !(line.contains("mailto:")
            || line.contains("://")
            || line.contains("www.")
            || line.contains("file:"))
        {
            return;
        }
        if line.contains("file:") {
            for m in FILE_URL.find_iter(line) {
                out.push(Link {
                    range: m.range(),
                    target: url_target(m.as_str()),
                });
            }
        }
        let potential = ["www", "http", "mailto", "ftp", "news"]
            .iter()
            .any(|w| line.contains(w));
        if !potential {
            return;
        }
        let mut at = 0;
        while let Some(m) = URL.find_at(line, at) {
            if m.as_str().starts_with("www.") && !www_may_start(line, m.start()) {
                // (?<![\p{L}0-9_.])www\. fails here; search on.
                at = m.start() + 1;
                continue;
            }
            out.push(Link {
                range: m.range(),
                target: url_target(m.as_str()),
            });
            at = m.end();
        }
    }
}

/// The look-behind `(?<![\p{L}0-9_.])` before `www.`.
fn www_may_start(line: &str, start: usize) -> bool {
    match line[..start].chars().next_back() {
        Some(c) => !(c.is_alphabetic() || c.is_ascii_digit() || c == '_' || c == '.'),
        None => true,
    }
}

/// `UrlFilter.buildHyperlinkInfo`: a `file:` URL to a file link, anything
/// else to a URL.
fn url_target(url: &str) -> LinkTarget {
    let prefix = if url.ends_with(".html") {
        None
    } else if url.starts_with("file:///") {
        Some("file://")
    } else if url.starts_with("file:") && !url.starts_with("file://") {
        Some("file:")
    } else {
        None
    };
    let Some(prefix) = prefix else {
        return LinkTarget::Url(url.to_owned());
    };
    let mut line = None;
    let mut column = None;
    let mut path_end = url.len();
    if let Some(last) = url.rfind(':')
        && last > prefix.len()
        && last < url.len() - 1
        && let Some(last_value) = parse_int(&url[last + 1..])
    {
        line = positive(last_value);
        path_end = last;
        if let Some(pre) = url[..last].rfind(':')
            && pre > prefix.len()
            && let Some(pre_value) = parse_int(&url[pre + 1..last])
        {
            line = positive(pre_value);
            column = positive(last_value);
            path_end = pre;
        }
    }
    let path = to_windows_path(url_decode(&url[prefix.len()..path_end]));
    LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    }
}

/// `Integer.parseInt`: an optional sign and decimal digits.
fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i64 = s.parse().ok()?;
    (i64::from(i32::MIN)..=i64::from(i32::MAX))
        .contains(&value)
        .then_some(value)
}

fn positive(value: i64) -> Option<u32> {
    u32::try_from(value).ok().filter(|v| *v > 0)
}

/// `/C:/x` to `C:/x`.
fn to_windows_path(path: String) -> String {
    let b = path.as_bytes();
    if b.len() >= 4
        && b[0] == b'/'
        && b[1].is_ascii_alphabetic()
        && b[2] == b':'
        && (b[3] == b'/' || b[3] == b'\\')
    {
        path[1..].to_owned()
    } else {
        path
    }
}

/// `URLDecoder.decode(s, UTF_8)`: `+` is a space, `%XX` a byte; a malformed
/// escape keeps the text as it is.
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => return s.to_owned(),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(line: &str) -> Vec<(String, LinkTarget)> {
        let mut out = Vec::new();
        UrlFilter.links(line, &mut out);
        out.into_iter()
            .map(|l| (line[l.range].to_owned(), l.target))
            .collect()
    }

    fn url(s: &str) -> LinkTarget {
        LinkTarget::Url(s.to_owned())
    }

    fn file(path: &str, line: Option<u32>, column: Option<u32>) -> LinkTarget {
        LinkTarget::File {
            path: PathBuf::from(path),
            line,
            column,
        }
    }

    #[test]
    fn web_urls() {
        assert_eq!(
            links("see https://docs.python.org/3/library/os.html#os.path, done."),
            vec![(
                "https://docs.python.org/3/library/os.html#os.path".into(),
                url("https://docs.python.org/3/library/os.html#os.path")
            )]
        );
        assert_eq!(
            links("Running on http://127.0.0.1:5000 (Press CTRL+C to quit)"),
            vec![("http://127.0.0.1:5000".into(), url("http://127.0.0.1:5000"))]
        );
        assert_eq!(
            links("mail mailto:someone@example.org or ftp://host/a.txt."),
            vec![
                (
                    "mailto:someone@example.org".into(),
                    url("mailto:someone@example.org")
                ),
                ("ftp://host/a.txt".into(), url("ftp://host/a.txt")),
            ]
        );
        assert_eq!(links("no links here"), vec![]);
    }

    #[test]
    fn www_needs_a_boundary() {
        assert_eq!(
            links("go to www.example.com now"),
            vec![("www.example.com".into(), url("www.example.com"))]
        );
        assert_eq!(links("awww.example.com"), vec![]);
        assert_eq!(links("x.www.example.com"), vec![]);
        assert_eq!(links("éwww.example.com"), vec![]);
        assert_eq!(
            links("(www.example.com)"),
            vec![("www.example.com".into(), url("www.example.com"))]
        );
    }

    #[test]
    fn file_urls() {
        assert_eq!(
            links("at file:///tmp/a%20b.py:12:5 here"),
            vec![(
                "file:///tmp/a%20b.py:12:5".into(),
                file("/tmp/a b.py", Some(12), Some(5))
            )]
        );
        assert_eq!(
            links("file:///C:/work/x.py:3"),
            vec![(
                "file:///C:/work/x.py:3".into(),
                file("C:/work/x.py", Some(3), None)
            )]
        );
        assert_eq!(
            links("file:/tmp/x.py"),
            vec![("file:/tmp/x.py".into(), file("/tmp/x.py", None, None))]
        );
        assert_eq!(
            links("report file:///tmp/r.html."),
            vec![("file:///tmp/r.html".into(), url("file:///tmp/r.html"))]
        );
    }
}
