use std::path::PathBuf;

use console::{Link, LinkFilter};

/// The reference's Python traceback filter (`PythonTracebackFilter` with the
/// `PyTestTracebackParser` and `PyTracebackParser` parsers): links
/// `File "<path>", line <n>` and pytest's `<path>:<n>` to the file at the
/// line when the file exists. A relative path is resolved against the
/// working directory, `~` against the home folder; a file whose extension is
/// not `py` gets no link (`File "<string>"` and similar never exist and get
/// none either). At most one link per line.
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
}

impl LinkFilter for TracebackFilter {
    fn links(&self, line: &str, out: &mut Vec<Link>) {
        let _ = (line, out, &self.working_dir);
        todo!("TracebackFilter::links")
    }
}
