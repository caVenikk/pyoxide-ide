use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

/// A process to start: program, arguments, working directory and
/// environment.
///
/// The `Debug` output lists the names of the environment variables only;
/// their values may be secrets and never appear in logs.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandLine {
    /// The executable. A bare name is looked up in `PATH`.
    pub program: PathBuf,
    /// The arguments, passed as they are (no shell is involved).
    pub args: Vec<OsString>,
    /// The working directory; `None` keeps the IDE's own.
    pub cwd: Option<PathBuf>,
    /// Applied in order over the inherited (or empty) environment: `Some`
    /// sets a variable, `None` removes it. A later entry for the same name
    /// wins.
    pub env: Vec<(OsString, Option<OsString>)>,
    /// `true` starts from the IDE's environment, `false` from an empty one
    /// (the reference's "Include system environment variables" turned off).
    pub inherit_env: bool,
}

impl CommandLine {
    /// A command line for `program` without arguments, inheriting the
    /// environment and the working directory.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            inherit_env: true,
        }
    }

    /// Adds an argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory.
    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Sets an environment variable.
    pub fn with_env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), Some(value.into())));
        self
    }

    /// Removes an environment variable.
    pub fn without_env(mut self, name: impl Into<OsString>) -> Self {
        self.env.push((name.into(), None));
        self
    }

    /// The command line as the reference prints it as the first console line
    /// (`TargetedCommandLine.getCommandPresentation` of IntelliJ Platform):
    /// the program and the arguments separated by spaces, quoted the way the
    /// reference quotes them for the current OS. On Unix the program is quoted
    /// when it contains whitespace and the arguments are not quoted at all; on
    /// Windows arguments with whitespace or special characters are quoted with
    /// the C runtime's rules. Never contains environment values.
    pub fn display(&self) -> String {
        presentation(&self.program_string(), &self.arg_strings(), cfg!(windows))
    }

    fn program_string(&self) -> String {
        self.program.as_os_str().to_string_lossy().into_owned()
    }

    fn arg_strings(&self) -> Vec<String> {
        self.args.iter().map(|a| lossy(a)).collect()
    }
}

impl fmt::Debug for CommandLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(name, value)| match value {
                Some(_) => format!("{}=<set>", lossy(name)),
                None => format!("{}=<removed>", lossy(name)),
            })
            .collect();
        f.debug_struct("CommandLine")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env", &env)
            .field("inherit_env", &self.inherit_env)
            .finish()
    }
}

fn lossy(s: &OsStr) -> String {
    s.to_string_lossy().into_owned()
}

/// `CommandLineUtil.toCommandLine(ParametersListUtil.escape(exe), params,
/// platform)` joined with spaces.
pub(crate) fn presentation(program: &str, args: &[String], windows: bool) -> String {
    let separator = if windows { '\\' } else { '/' };
    let mut out = escape_program(program)
        .chars()
        .map(|c| if c == '/' || c == '\\' { separator } else { c })
        .collect::<String>();
    for arg in args {
        out.push(' ');
        if windows {
            out.push_str(&windows_quote(arg));
        } else {
            out.push_str(arg);
        }
    }
    out
}

/// `ParametersListUtil.escape`: backslash-escapes double quotes and quotes
/// the whole when it is empty or contains whitespace or `|`.
fn escape_program(program: &str) -> String {
    let mut s = program.replace('"', "\\\"");
    if s.is_empty() || s.chars().any(is_java_whitespace) || s.contains('|') {
        let trailing = s.len() - s.trim_end_matches('\\').len();
        s = format!("\"{s}\"");
        s.push_str(&"\\".repeat(trailing));
    }
    s
}

/// `CommandLineUtil.backslashEscapeQuotes`: the C runtime's quoting of one
/// argument (not for `cmd.exe`).
fn windows_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_owned();
    }
    let mut result = String::with_capacity(arg.len() + 2);
    let chars: Vec<char> = arg.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let start = i;
            while i < chars.len() && chars[i] == '\\' {
                i += 1;
            }
            let run = i - start;
            let doubled = i == chars.len() || chars[i] == '"';
            let count = if doubled { run * 2 } else { run };
            result.extend(std::iter::repeat_n('\\', count));
            continue;
        }
        if chars[i] == '"' {
            result.push_str("\\\"");
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    let special = arg.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t' | '"' | '*' | '?' | '[' | '{' | '}' | '~' | '(' | ')' | '\''
        )
    });
    if result != arg || special {
        let quoted = result.len() >= 2 && result.starts_with('"') && result.ends_with('"');
        if !quoted {
            result = format!("\"{result}\"");
        }
    }
    result
}

/// `Character.isWhitespace` of Java: no-break spaces and NEL are not
/// whitespace there.
fn is_java_whitespace(c: char) -> bool {
    matches!(c, '\u{1C}'..='\u{1F}')
        || (c.is_whitespace() && !matches!(c, '\u{A0}' | '\u{2007}' | '\u{202F}' | '\u{85}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn unix_presentation_quotes_only_the_program() {
        assert_eq!(
            presentation("/p/.venv/bin/python", &s(&["/p/main.py"]), false),
            "/p/.venv/bin/python /p/main.py"
        );
        assert_eq!(
            presentation(
                "/My Project/bin/python",
                &s(&["/My Project/a b.py", ""]),
                false
            ),
            "\"/My Project/bin/python\" /My Project/a b.py "
        );
    }

    #[test]
    fn windows_presentation_follows_the_c_runtime_rules() {
        assert_eq!(
            presentation(
                "C:/p/.venv/Scripts/python.exe",
                &s(&["C:\\p\\main.py"]),
                true
            ),
            "C:\\p\\.venv\\Scripts\\python.exe C:\\p\\main.py"
        );
        assert_eq!(windows_quote("a b"), "\"a b\"");
        assert_eq!(windows_quote(""), "\"\"");
        assert_eq!(windows_quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(windows_quote("C:\\dir\\"), "\"C:\\dir\\\\\"");
        assert_eq!(windows_quote("C:\\dir\\x"), "C:\\dir\\x");
        assert_eq!(windows_quote("*.py"), "\"*.py\"");
    }

    #[test]
    fn debug_hides_environment_values() {
        let cmd = CommandLine::new("python")
            .with_env("TOKEN", "secret-value")
            .without_env("OLD");
        let text = format!("{cmd:?}");
        assert!(text.contains("TOKEN=<set>"));
        assert!(text.contains("OLD=<removed>"));
        assert!(!text.contains("secret-value"));
    }
}
