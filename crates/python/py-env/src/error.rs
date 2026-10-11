use std::fmt::Write as _;

use serde::Serialize;

/// What went wrong, coarse enough for a front-end to decide how to react
/// (offer to install uv, show a "no network" banner, highlight the package).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// A required tool (uv, pip, a Python interpreter, poetry, ...) is not installed.
    ToolMissing,
    Timeout,
    Cancelled,
    /// No interpreter or environment matches the request.
    NoInterpreter,
    /// There is no `pyproject.toml` where one is needed.
    NoProject,
    /// The resolver found no consistent set of versions.
    Resolution,
    /// The index or a download could not be reached.
    Network,
    /// A package does not exist on the index or in the project.
    PackageNotFound,
    /// The thing to create is already there.
    AlreadyExists,
    InvalidInput,
    /// A project file or a tool's output could not be understood.
    Parse,
    Io,
    /// The tool ran and failed for another reason.
    Failed,
}

impl ErrorKind {
    /// Stable code used by the `env.*` commands.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::ToolMissing => "env.tool_missing",
            ErrorKind::Timeout => "env.timeout",
            ErrorKind::Cancelled => "env.cancelled",
            ErrorKind::NoInterpreter => "env.no_interpreter",
            ErrorKind::NoProject => "env.no_project",
            ErrorKind::Resolution => "env.resolution",
            ErrorKind::Network => "env.network",
            ErrorKind::PackageNotFound => "env.package_not_found",
            ErrorKind::AlreadyExists => "env.already_exists",
            ErrorKind::InvalidInput => "invalid_args",
            ErrorKind::Parse => "env.parse",
            ErrorKind::Io => "env.io",
            ErrorKind::Failed => "env.failed",
        }
    }
}

/// Failure of an environment operation. `message` is short and readable;
/// `detail` carries the condensed error output of the tool for a log pane.
/// Credentials embedded in URLs are redacted from both.
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[error("{message}")]
pub struct EnvError {
    pub kind: ErrorKind,
    pub message: String,
    /// The tool that failed (`uv`, `pip`, `python`, ...).
    pub tool: Option<String>,
    pub exit_code: Option<i32>,
    pub detail: Option<String>,
}

pub type Result<T, E = EnvError> = std::result::Result<T, E>;

impl EnvError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: redact_credentials(&message.into()),
            tool: None,
            exit_code: None,
            detail: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    pub fn io(context: impl std::fmt::Display, e: &std::io::Error) -> Self {
        Self::new(ErrorKind::Io, format!("{context}: {e}"))
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, message)
    }

    pub fn tool_missing(tool: &str, hint: &str) -> Self {
        let mut e = Self::new(
            ErrorKind::ToolMissing,
            format!("{tool} is not installed. {hint}"),
        );
        e.tool = Some(tool.to_owned());
        e
    }

    pub fn with_tool(mut self, tool: &str) -> Self {
        self.tool = Some(tool.to_owned());
        self
    }

    /// Map a finished process that did not succeed to a typed error. `stderr`
    /// is the (possibly truncated) error output of the tool.
    pub fn from_failure(tool: &str, exit_code: Option<i32>, stderr: &str) -> Self {
        let stderr = redact_credentials(stderr);
        let kind = classify(&stderr);
        let message = summarize(tool, exit_code, &stderr);
        Self {
            kind,
            message,
            tool: Some(tool.to_owned()),
            exit_code,
            detail: Some(condense(&stderr, DETAIL_LINES)).filter(|d| !d.is_empty()),
        }
    }
}

impl From<std::io::Error> for EnvError {
    fn from(e: std::io::Error) -> Self {
        EnvError::new(ErrorKind::Io, e.to_string())
    }
}

impl From<EnvError> for commands::CommandError {
    fn from(e: EnvError) -> Self {
        let message = match &e.detail {
            // The summary is enough for a one-line toast; the log pane reads `detail`.
            Some(d) if e.kind == ErrorKind::Resolution || e.kind == ErrorKind::Failed => {
                format!("{}\n{d}", e.message)
            }
            _ => e.message.clone(),
        };
        commands::CommandError::new(e.kind.code(), message)
    }
}

const DETAIL_LINES: usize = 40;
const MESSAGE_LINES: usize = 6;

fn classify(stderr: &str) -> ErrorKind {
    let s = stderr.to_ascii_lowercase();
    let has = |needle: &str| s.contains(needle);
    // Order matters: a missing package also prints "no solution found".
    if has("was not found in the package registry")
        || has("could not be found in `project.")
        || has("could not be found in `dependency-groups")
        || has("could not find a version that satisfies")
        || has("no matching distribution found")
    {
        ErrorKind::PackageNotFound
    } else if has("network connectivity is disabled")
        || has("network was disabled")
        || has("failed to fetch")
        || has("error sending request")
        || has("dns error")
        || has("connection refused")
        || has("network is unreachable")
        || has("failed to download")
        || has("temporary failure in name resolution")
        || has("tls handshake")
        || has("connection timed out")
        || has("max retries exceeded")
        || has("could not connect")
    {
        ErrorKind::Network
    } else if has("no solution found") || has("resolutionimpossible") || has("dependency conflict")
    {
        ErrorKind::Resolution
    } else if has("no interpreter found")
        || has("no download found")
        || has("no python installation found")
        || has("no virtual environment found")
        || has("no module named pip")
        || has("failed to inspect python interpreter")
        || has("python executable does not exist")
    {
        ErrorKind::NoInterpreter
    } else if has("no `pyproject.toml` found") || has("could not find `pyproject.toml`") {
        ErrorKind::NoProject
    } else if has("already exists") || has("already initialized") {
        ErrorKind::AlreadyExists
    } else if has("failed to parse") || has("invalid `pyproject.toml`") || has("toml parse error") {
        ErrorKind::Parse
    } else {
        ErrorKind::Failed
    }
}

/// One to a few readable lines: the tool's own `error:` lines and what they
/// say about the cause, without version lists and hints.
fn summarize(tool: &str, exit_code: Option<i32>, stderr: &str) -> String {
    let lines: Vec<String> = clean_lines(stderr);
    let mut picked: Vec<&str> = Vec::new();
    for l in &lines {
        let t = l.trim();
        let wanted = t.starts_with("error:")
            || t.starts_with("ERROR:")
            || t.starts_with('×')
            || t.starts_with("╰─▶")
            || t.starts_with("Caused by:")
            || t.starts_with("Because ")
            || t.starts_with("And because ")
            || t.starts_with("hint:");
        if wanted {
            picked.push(t);
        }
        if picked.len() >= MESSAGE_LINES {
            break;
        }
    }
    if picked.is_empty() {
        picked = lines
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
    }
    let mut out = String::new();
    for (i, l) in picked.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l.strip_prefix("error: ").unwrap_or(l));
    }
    if out.is_empty() {
        out = match exit_code {
            Some(c) => format!("{tool} exited with status {c}"),
            None => format!("{tool} was terminated by a signal"),
        };
    }
    truncate_chars(&out, 800)
}

/// Non-empty lines of tool output without the long enumerations of versions
/// that resolvers print ("requests==2.1.0").
fn clean_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim_end().to_owned())
        .filter(|l| !l.trim().is_empty() && !is_version_enumeration(l))
        .collect()
}

fn is_version_enumeration(line: &str) -> bool {
    let t = line.trim();
    let Some(pos) = t.find(['=', '<', '>']) else {
        return false;
    };
    let (name, spec) = t.split_at(pos);
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && spec
            .trim_start_matches(['=', '<', '>', '!', '~'])
            .starts_with(|c: char| c.is_ascii_digit())
        && !spec.contains(' ')
}

/// Condensed error output: no enumerations, at most `max_lines` lines.
pub(crate) fn condense(text: &str, max_lines: usize) -> String {
    let lines = clean_lines(text);
    let mut out = String::new();
    let shown = if lines.len() > max_lines {
        let _ = writeln!(
            out,
            "... ({} earlier lines omitted)",
            lines.len() - max_lines
        );
        &lines[lines.len() - max_lines..]
    } else {
        &lines[..]
    };
    out.push_str(&shown.join("\n"));
    truncate_chars(&out, 8000)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("...");
    out
}

/// Replace `user:password@` in URLs with `***@`.
pub fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '"' | '\'' | '`' | '<' | '>'))
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_url_credentials() {
        assert_eq!(
            redact_credentials("fetch https://user:s3cret@example.org/simple/ failed"),
            "fetch https://***@example.org/simple/ failed"
        );
        assert_eq!(
            redact_credentials("https://token@host and http://plain.example/x"),
            "https://***@host and http://plain.example/x"
        );
        assert_eq!(redact_credentials("no urls here"), "no urls here");
    }

    #[test]
    fn classifies_known_failures() {
        let e = EnvError::from_failure(
            "uv",
            Some(2),
            "error: The dependency `nothere` could not be found in `project.dependencies`\n",
        );
        assert_eq!(e.kind, ErrorKind::PackageNotFound);
        assert_eq!(
            e.message,
            "The dependency `nothere` could not be found in `project.dependencies`"
        );

        let e = EnvError::from_failure(
            "uv",
            Some(2),
            "error: No interpreter found for Python 3.99 in managed installations or search path\n\nhint: uv embeds available Python downloads\n",
        );
        assert_eq!(e.kind, ErrorKind::NoInterpreter);
        assert!(e.message.contains("Python 3.99"));

        let e = EnvError::from_failure(
            "uv",
            Some(1),
            "error: Failed to install cpython-3.11\n  Caused by: Failed to download https://example.org/x.tar.gz\n  Caused by: Network connectivity is disabled, but the requested data wasn't found in the cache\n",
        );
        assert_eq!(e.kind, ErrorKind::Network);

        let e = EnvError::from_failure(
            "uv",
            Some(2),
            "error: Failed to create virtual environment\n  Caused by: A virtual environment already exists at: .venv\n\nhint: Use the `--clear` flag\n",
        );
        assert_eq!(e.kind, ErrorKind::AlreadyExists);

        let e = EnvError::from_failure(
            "uv",
            Some(2),
            "error: No `pyproject.toml` found in current directory or any parent directory\n",
        );
        assert_eq!(e.kind, ErrorKind::NoProject);

        let e = EnvError::from_failure("pip", Some(1), "ERROR: No module named pip\n");
        assert_eq!(e.kind, ErrorKind::NoInterpreter);

        let e = EnvError::from_failure("pip", Some(1), "boom\n");
        assert_eq!(e.kind, ErrorKind::Failed);
        assert_eq!(e.message, "boom");

        let e = EnvError::from_failure("uv", None, "");
        assert_eq!(e.message, "uv was terminated by a signal");
    }

    #[test]
    fn resolution_failure_drops_version_lists() {
        let mut stderr = String::from(
            "  × No solution found when resolving dependencies:\n  ╰─▶ Because only the following versions of requests are available:\n",
        );
        for i in 0..200 {
            let _ = writeln!(stderr, "          requests==2.{i}.0");
        }
        stderr.push_str(
            "      and requests<=2.31.0 conflicts with other requirements, we can conclude that requests<=2.31.0 cannot be used.\n",
        );
        let e = EnvError::from_failure("uv", Some(1), &stderr);
        assert_eq!(e.kind, ErrorKind::Resolution);
        assert!(!e.message.contains("requests==2.1.0"));
        assert!(e.message.contains("No solution found"));
        let detail = e.detail.unwrap();
        assert!(!detail.contains("requests==2.100.0"));
        assert!(detail.contains("cannot be used"));
    }

    #[test]
    fn offline_resolution_failures_are_network_errors() {
        let e = EnvError::from_failure(
            "uv",
            Some(1),
            "  × No solution found when resolving dependencies:\n  ╰─▶ Because requests was not found in the cache and your project depends\n      on requests, we can conclude that your project's requirements are\n      unsatisfiable.\n\nhint: Packages were unavailable because the network was disabled.\n",
        );
        assert_eq!(e.kind, ErrorKind::Network);
        assert!(
            e.message.starts_with("× No solution found"),
            "{}",
            e.message
        );
        assert!(e.message.contains("not found in the cache"));
    }

    #[test]
    fn missing_python_downloads_are_no_interpreter() {
        let e = EnvError::from_failure(
            "uv",
            Some(2),
            "error: No download found for request: cpython-3.99-macos-aarch64-none\n",
        );
        assert_eq!(e.kind, ErrorKind::NoInterpreter);
    }

    #[test]
    fn version_enumeration_detection() {
        assert!(is_version_enumeration("          requests==2.1.0"));
        assert!(is_version_enumeration("charset-normalizer<=2.0.0"));
        assert!(!is_version_enumeration("hint: use --offline=false"));
        assert!(!is_version_enumeration("a = b"));
        assert!(!is_version_enumeration("x==y"));
    }

    #[test]
    fn command_error_codes_are_stable() {
        let c: commands::CommandError = EnvError::new(ErrorKind::Timeout, "slow").into();
        assert_eq!(c.code, "env.timeout");
        let c: commands::CommandError = EnvError::invalid("bad").into();
        assert_eq!(c.code, "invalid_args");
    }
}
