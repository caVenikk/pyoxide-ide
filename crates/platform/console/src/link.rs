use std::ops::Range;
use std::path::PathBuf;

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
    /// A URL to open in the browser.
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

/// The reference's URL filter (`UrlFilter` with `URLUtil` patterns):
/// `http(s)://`, `ftp(s)://`, `news://`, `mailto:` and `www.` links, and
/// `file:` URLs, which become file links with an optional `:line[:column]`
/// suffix.
#[derive(Debug, Clone, Copy, Default)]
pub struct UrlFilter;

impl LinkFilter for UrlFilter {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        let _ = (line, out);
        todo!("UrlFilter::links")
    }
}
