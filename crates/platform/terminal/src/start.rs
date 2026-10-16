//! How a session starts, as in the reference (`LocalOptionsConfigurer`,
//! `LocalTerminalStartCommandBuilder` and
//! `TerminalProjectOptionsProvider.findDefaultShellPath` of the IntelliJ
//! terminal plugin): the shell and its options, the working directory and
//! the environment.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

/// The variable that names the terminal emulator to the programs in it
/// (the reference sets `TERMINAL_EMULATOR=JetBrains-JediTerm`).
pub const TERMINAL_EMULATOR_VAR: &str = "TERMINAL_EMULATOR";
/// Pyoxide's value of [`TERMINAL_EMULATOR_VAR`].
pub const TERMINAL_EMULATOR_NAME: &str = "Pyoxide";
/// A random id per session (as the reference sets it).
pub const TERM_SESSION_ID_VAR: &str = "TERM_SESSION_ID";

/// The operating system the rules are evaluated for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// The kind of shell, by its executable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    Sh,
    /// `pwsh` or Windows PowerShell (`powershell`).
    PowerShell,
    /// `cmd.exe`.
    Cmd,
    Other,
}

impl ShellKind {
    /// The kind of the shell `program` (a path or a name).
    pub fn of(program: &str) -> Self {
        let name = file_name(program);
        let lower = name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        match stem {
            "powershell" | "pwsh" => return ShellKind::PowerShell,
            "cmd" => return ShellKind::Cmd,
            _ => {}
        }
        // Unix shell names are case-sensitive in the reference (`ShellNameUtil`);
        // Git Bash on Windows is `bash.exe`.
        match name.strip_suffix(".exe").unwrap_or(name) {
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            "sh" => ShellKind::Sh,
            _ => ShellKind::Other,
        }
    }
}

/// The last component of a path written with `/` or `\`.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A program with its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ShellCommand {
    pub fn kind(&self) -> ShellKind {
        ShellKind::of(&self.program)
    }
}

/// The shell used when the settings name none: `powershell.exe` on Windows;
/// elsewhere `$SHELL` if it exists, else `/bin/bash`, else `/bin/sh`.
pub fn default_shell_path() -> String {
    default_shell_path_for(Os::current(), std::env::var("SHELL").ok().as_deref(), |p| {
        p.exists()
    })
}

/// [`default_shell_path`] for an OS, a `$SHELL` value and a file check.
pub fn default_shell_path_for(
    os: Os,
    shell_var: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> String {
    if os == Os::Windows {
        return "powershell.exe".into();
    }
    if let Some(shell) = shell_var.filter(|s| !s.is_empty() && exists(Path::new(s))) {
        return shell.into();
    }
    if exists(Path::new("/bin/bash")) {
        return "/bin/bash".into();
    }
    "/bin/sh".into()
}

/// The command for a shell path from the settings
/// (`LocalTerminalStartCommandBuilder.convertShellPathToCommand`): an
/// existing absolute file is the program as it is, anything else is split
/// into a command line; `--login` is added for bash, zsh, fish (and sh) on
/// macOS and `-i` for bash, zsh and fish (and sh on macOS), unless the
/// command has `--login`, `-l` or `-i` already.
pub fn shell_command(shell_path: &str) -> ShellCommand {
    shell_command_for(Os::current(), shell_path, |p| p.is_file())
}

/// [`shell_command`] for an OS and a file check.
pub fn shell_command_for(
    os: Os,
    shell_path: &str,
    is_file: impl Fn(&Path) -> bool,
) -> ShellCommand {
    let path = Path::new(shell_path);
    let mut parts = if is_absolute(os, shell_path) && is_file(path) {
        vec![shell_path.to_owned()]
    } else {
        split_command_line(shell_path, os != Os::Windows)
    };
    if parts.is_empty() {
        return ShellCommand {
            program: String::new(),
            args: Vec::new(),
        };
    }
    let program = parts.remove(0);
    let mut args = parts;
    let name = file_name(&program);
    let bash_zsh_fish = matches!(name, "bash" | "zsh" | "fish");
    let mac_sh = name == "sh" && os == Os::MacOs;
    let has_option = args
        .iter()
        .any(|a| a == "--login" || a == "-l" || a == "-i");
    if !has_option {
        if (bash_zsh_fish || mac_sh) && os == Os::MacOs {
            args.push("--login".into());
        }
        if bash_zsh_fish || mac_sh {
            args.push("-i".into());
        }
    }
    ShellCommand { program, args }
}

fn is_absolute(os: Os, path: &str) -> bool {
    if os == Os::Windows {
        let b = path.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
            || path.starts_with("\\\\")
    } else {
        path.starts_with('/')
    }
}

/// Splits a command line at blanks outside quotes, removing the quotes
/// (`ParametersListUtil.parse` without keeping quotes); single quotes count
/// only when `single_quotes` is set (not on Windows).
pub fn split_command_line(line: &str, single_quotes: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {
                if c == '\\' && quote == Some('"') && chars.peek() == Some(&'"') {
                    current.push(chars.next().unwrap_or('"'));
                } else {
                    current.push(c);
                }
            }
            None if c == '"' || (single_quotes && c == '\'') => {
                quote = Some(c);
                in_token = true;
            }
            None if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None => {
                current.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        out.push(current);
    }
    out
}

/// Where a new session starts (`LocalOptionsConfigurer`): the requested
/// folder (a file stands for its folder), else the "Start directory"
/// setting, else the product's default folder, else the project root, else
/// the home folder. Folders that do not exist are skipped.
pub fn working_directory(
    requested: Option<&Path>,
    start_directory: Option<&Path>,
    default_directory: Option<&Path>,
    project_root: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = requested.filter(|p| p.is_absolute()) {
        if path.is_dir() {
            return Some(path.to_owned());
        }
        if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            return Some(parent.to_owned());
        }
    }
    [start_directory, default_directory]
        .into_iter()
        .flatten()
        .find(|p| p.is_dir())
        .or(project_root)
        .or(home)
        .map(Path::to_owned)
}

/// The environment variables a session sets over the inherited ones
/// (`LocalOptionsConfigurer.getTerminalEnvironment`), later ones winning:
/// `base` (the product's and the integration's variables), `TERM` (not on
/// Windows), `COLORTERM`, the emulator name, a session id, `LC_CTYPE` on
/// macOS, then the project's own variables. On Windows names are
/// case-insensitive.
pub fn environment(
    os: Os,
    base: &[(String, String)],
    project_variables: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env = EnvList::new(os);
    for (name, value) in base {
        env.set(name, value);
    }
    if os != Os::Windows {
        env.set("TERM", "xterm-256color");
    }
    env.set("COLORTERM", "truecolor");
    env.set(TERMINAL_EMULATOR_VAR, TERMINAL_EMULATOR_NAME);
    env.set(TERM_SESSION_ID_VAR, &session_id());
    if os == Os::MacOs {
        // The reference's default character encoding (UTF-8).
        env.set("LC_CTYPE", "UTF-8");
    }
    for (name, value) in project_variables {
        env.set(name, value);
    }
    env.vars
}

/// Variables in the order they were first set; names compare
/// case-insensitively on Windows.
struct EnvList {
    insensitive: bool,
    vars: Vec<(String, String)>,
}

impl EnvList {
    fn new(os: Os) -> Self {
        Self {
            insensitive: os == Os::Windows,
            vars: Vec::new(),
        }
    }

    fn set(&mut self, name: &str, value: &str) {
        let insensitive = self.insensitive;
        let existing = self.vars.iter_mut().find(|(n, _)| {
            if insensitive {
                n.eq_ignore_ascii_case(name)
            } else {
                n == name
            }
        });
        match existing {
            Some(entry) => entry.1 = value.to_owned(),
            None => self.vars.push((name.to_owned(), value.to_owned())),
        }
    }
}

/// A random id in the form of a UUID (version 4).
fn session_id() -> String {
    let random = |salt: u64| {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        h.finish()
    };
    let (a, b) = (random(1), random(2));
    let bytes: Vec<u8> = a.to_be_bytes().into_iter().chain(b.to_be_bytes()).collect();
    let mut bytes: [u8; 16] = bytes.try_into().unwrap_or([0; 16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(cmd: &ShellCommand) -> Vec<&str> {
        std::iter::once(cmd.program.as_str())
            .chain(cmd.args.iter().map(String::as_str))
            .collect()
    }

    #[test]
    fn login_and_interactive_options() {
        let file = |_: &Path| true;
        let cmd = shell_command_for(Os::MacOs, "/bin/zsh", file);
        assert_eq!(args(&cmd), ["/bin/zsh", "--login", "-i"]);
        let cmd = shell_command_for(Os::Linux, "/bin/bash", file);
        assert_eq!(args(&cmd), ["/bin/bash", "-i"]);
        let cmd = shell_command_for(Os::MacOs, "/bin/sh", file);
        assert_eq!(args(&cmd), ["/bin/sh", "--login", "-i"]);
        let cmd = shell_command_for(Os::Linux, "/bin/sh", file);
        assert_eq!(args(&cmd), ["/bin/sh"]);
        let cmd = shell_command_for(Os::MacOs, "/opt/homebrew/bin/fish -l", |_| false);
        assert_eq!(args(&cmd), ["/opt/homebrew/bin/fish", "-l"]);
        let cmd = shell_command_for(Os::Linux, "/usr/bin/pwsh", file);
        assert_eq!(args(&cmd), ["/usr/bin/pwsh"]);
        let cmd = shell_command_for(Os::Windows, "powershell.exe", |_| false);
        assert_eq!(args(&cmd), ["powershell.exe"]);
        let cmd = shell_command_for(
            Os::Windows,
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
            file,
        );
        assert_eq!(args(&cmd), ["C:\\Program Files\\PowerShell\\7\\pwsh.exe"]);
        let cmd = shell_command_for(
            Os::Windows,
            "\"C:\\Program Files\\Git\\bin\\bash.exe\" --login",
            |_| false,
        );
        assert_eq!(
            args(&cmd),
            ["C:\\Program Files\\Git\\bin\\bash.exe", "--login"]
        );
        let cmd = shell_command_for(Os::Windows, "wsl.exe --distribution Ubuntu", |_| false);
        assert_eq!(args(&cmd), ["wsl.exe", "--distribution", "Ubuntu"]);
    }

    #[test]
    fn command_line_splitting() {
        assert_eq!(
            split_command_line("a 'b c' \"d e\"  f", true),
            ["a", "b c", "d e", "f"]
        );
        assert_eq!(split_command_line("a 'b c'", false), ["a", "'b", "c'"]);
        assert_eq!(split_command_line("\"\" x", true), ["", "x"]);
        assert!(split_command_line("   ", true).is_empty());
    }

    #[test]
    fn default_shell() {
        let none = |_: &Path| false;
        let all = |_: &Path| true;
        assert_eq!(
            default_shell_path_for(Os::Windows, Some("/bin/zsh"), all),
            "powershell.exe"
        );
        assert_eq!(
            default_shell_path_for(Os::MacOs, Some("/bin/zsh"), all),
            "/bin/zsh"
        );
        assert_eq!(
            default_shell_path_for(Os::Linux, Some("/gone"), |p| p != Path::new("/gone")),
            "/bin/bash"
        );
        assert_eq!(default_shell_path_for(Os::Linux, None, none), "/bin/sh");
    }

    #[test]
    fn shell_kinds() {
        assert_eq!(ShellKind::of("/bin/zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::of("C:\\Git\\bin\\bash.exe"), ShellKind::Bash);
        assert_eq!(ShellKind::of("PowerShell.EXE"), ShellKind::PowerShell);
        assert_eq!(ShellKind::of("pwsh"), ShellKind::PowerShell);
        assert_eq!(
            ShellKind::of("C:\\Windows\\System32\\cmd.exe"),
            ShellKind::Cmd
        );
        assert_eq!(ShellKind::of("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(ShellKind::of("/bin/tcsh"), ShellKind::Other);
    }

    #[test]
    fn working_directory_rules() {
        let dir = std::env::temp_dir();
        let missing = dir.join("pyoxide-terminal-missing-dir");
        let file = dir.join("pyoxide-terminal-missing-dir").join("file.txt");
        assert_eq!(
            working_directory(Some(&dir), None, None, None, None),
            Some(dir.clone())
        );
        // A path inside an existing folder that is not a folder: its parent.
        let inside = dir.join("pyoxide-terminal-no-such-file.py");
        assert_eq!(
            working_directory(Some(&inside), None, None, None, None),
            Some(dir.clone())
        );
        // A requested folder that does not exist: the next rule.
        assert_eq!(
            working_directory(Some(&file), Some(&missing), Some(&dir), None, None),
            Some(dir.clone())
        );
        assert_eq!(
            working_directory(None, Some(&missing), None, Some(&missing), Some(&dir)),
            Some(missing.clone())
        );
        assert_eq!(
            working_directory(Some(Path::new("relative")), None, None, None, Some(&dir)),
            Some(dir)
        );
    }

    #[test]
    fn environment_order() {
        let base = vec![
            ("TERM".to_owned(), "dumb".to_owned()),
            ("VIRTUAL_ENV".to_owned(), "/p/.venv".to_owned()),
        ];
        let project = vec![("COLORTERM".to_owned(), "24bit".to_owned())];
        let env = environment(Os::MacOs, &base, &project);
        let get = |name: &str| env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        assert_eq!(get("TERM"), Some("xterm-256color"));
        assert_eq!(get("VIRTUAL_ENV"), Some("/p/.venv"));
        assert_eq!(get("COLORTERM"), Some("24bit"));
        assert_eq!(get(TERMINAL_EMULATOR_VAR), Some(TERMINAL_EMULATOR_NAME));
        assert_eq!(get("LC_CTYPE"), Some("UTF-8"));
        let id = get(TERM_SESSION_ID_VAR).unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");

        let env = environment(Os::Windows, &[], &[]);
        assert!(env.iter().all(|(n, _)| n != "TERM" && n != "LC_CTYPE"));
        assert_ne!(
            environment(Os::Linux, &[], &[]),
            environment(Os::Linux, &[], &[])
        );
    }
}
