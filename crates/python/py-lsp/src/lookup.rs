//! Finding a tool's executable, as the reference's lookup modes do
//! (`ExecutableDiscoveryMode`, `PyToolExt.getExecutableWithBaseArgs`):
//!
//! - Sdk: the folder of the project interpreter
//!   (`py_env::paths::env_tool`: `bin`, or `Scripts` on Windows), then the
//!   custom path, then `PATH` and the per-user tool folders, then `uvx`;
//! - Path: the custom path, then `PATH` and the per-user folders, then
//!   `uvx`;
//! - uvx: always `uvx <package>`.
//!
//! `uvx` itself is found on `PATH` and in the per-user folders. A custom
//! path that is not an executable file is passed over. Ending at `uvx`
//! needs [`LookupContext::allow_uvx`]: Pyoxide's default tools do not
//! download anything without asking (ADR 0010).
//!
//! The version check runs `<tool> --version` and reads the answer as the
//! reference does (`PyToolVersionCheck`).
//!
//! Everything here blocks on the file system (and [`probe_version`] on a
//! process): run it off the UI thread.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use process::{CommandLine, ProcessEvent};
use py_env::version::Version;
use serde::Serialize;

use crate::settings::Tool;

/// How a tool's executable is found (the reference's Lookup column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lookup {
    #[default]
    Sdk,
    Path,
    Uvx,
}

impl Lookup {
    /// The value in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Lookup::Sdk => "sdk",
            Lookup::Path => "path",
            Lookup::Uvx => "uvx",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "sdk" => Some(Lookup::Sdk),
            "path" => Some(Lookup::Path),
            "uvx" => Some(Lookup::Uvx),
            _ => None,
        }
    }

    /// The name the reference shows (`Sdk`, `Path`, `uvx`).
    pub fn label(self) -> &'static str {
        match self {
            Lookup::Sdk => "Sdk",
            Lookup::Path => "Path",
            Lookup::Uvx => "uvx",
        }
    }
}

/// Where tools are searched besides the interpreter's folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPaths {
    /// The folders of `PATH`, in order.
    pub path: Vec<PathBuf>,
    /// Per-user tool folders, searched after `PATH`.
    pub user_dirs: Vec<PathBuf>,
    /// The extensions a name is tried with (`PATHEXT` in lower case on
    /// Windows, `.exe` first by default); none elsewhere, where the bare
    /// name is the file.
    pub extensions: Vec<String>,
}

impl SearchPaths {
    /// The application's environment: `PATH`, the per-user folders of the
    /// reference (`~/.local/bin`; on Windows `%APPDATA%\Python\Scripts`
    /// and `%LOCALAPPDATA%\Python\Scripts`) and the folders installers put
    /// tools in that an application started outside a shell misses
    /// (`py_env::paths::extra_tool_dirs`).
    pub fn from_env() -> Self {
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let extensions = if cfg!(windows) {
            path_extensions(std::env::var("PATHEXT").ok().as_deref())
        } else {
            Vec::new()
        };
        Self {
            path,
            user_dirs: user_dirs(),
            extensions,
        }
    }

    /// The executable `name` in `PATH`, then in the per-user folders.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .chain(&self.user_dirs)
            .find_map(|dir| find_in(dir, name, &self.extensions))
    }
}

/// The extensions of a `PATHEXT` value, in lower case; Windows' default
/// list when it is not set.
pub fn path_extensions(pathext: Option<&str>) -> Vec<String> {
    pathext
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(".COM;.EXE;.BAT;.CMD")
        .split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn user_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
    }
    if cfg!(windows) {
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(base).join("Python").join("Scripts"));
            }
        }
    }
    for dir in py_env::paths::extra_tool_dirs() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// The executable `name` in `dir`: with each of `extensions` when there
/// are any and the name has none, else the bare name.
fn find_in(dir: &Path, name: &str, extensions: &[String]) -> Option<PathBuf> {
    if dir.as_os_str().is_empty() {
        return None;
    }
    if extensions.is_empty() || Path::new(name).extension().is_some() {
        let path = dir.join(name);
        return is_executable(&path).then_some(path);
    }
    extensions
        .iter()
        .map(|ext| dir.join(format!("{name}{ext}")))
        .find(|path| is_executable(path))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// What the lookup knows about the project.
#[derive(Debug, Clone, Copy, Default)]
pub struct LookupContext<'a> {
    /// The project interpreter.
    pub interpreter: Option<&'a Path>,
    /// The person's custom executable.
    pub custom_path: Option<&'a Path>,
    /// The lookup may end at `uvx` (the tool was enabled explicitly, or the
    /// person agreed to run it through uv).
    pub allow_uvx: bool,
    /// Where to search; the application's environment
    /// ([`SearchPaths::from_env`]) when `None`.
    pub search: Option<&'a SearchPaths>,
}

/// A found tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Executable {
    /// An executable file.
    Path(PathBuf),
    /// Run through `uvx`, which downloads the package when needed.
    Uvx {
        uvx: PathBuf,
        package: String,
        executable: String,
    },
}

impl Executable {
    /// A tool run through `uvx`.
    pub fn uvx(tool: Tool, uvx: PathBuf) -> Self {
        Executable::Uvx {
            uvx,
            package: tool.package().to_owned(),
            executable: tool.executable().to_owned(),
        }
    }

    /// The command line that runs it with `args`: `uvx <package>` when the
    /// executable is named like the package, else `uvx --from <package>
    /// <executable>`, as the reference runs it.
    pub fn command_line(&self, args: &[&str]) -> CommandLine {
        match self {
            Executable::Path(path) => CommandLine::new(path).args(args.iter().copied()),
            Executable::Uvx {
                uvx,
                package,
                executable,
            } => {
                let command = CommandLine::new(uvx);
                let command = if package == executable {
                    command.arg(package)
                } else {
                    command.args(["--from", package.as_str(), executable.as_str()])
                };
                command.args(args.iter().copied())
            }
        }
    }

    /// Runs through `uvx`.
    pub fn is_uvx(&self) -> bool {
        matches!(self, Executable::Uvx { .. })
    }
}

/// A step of the lookup chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// The folder of the project interpreter.
    Sdk,
    /// The person's custom executable.
    Custom,
    /// `PATH` and the per-user tool folders.
    Path,
    /// `uvx`, which runs the package (downloading it when needed).
    Uvx,
}

impl Stage {
    /// The name shown in the lookup summary.
    pub fn label(self) -> &'static str {
        match self {
            Stage::Sdk => "Sdk",
            Stage::Custom => "Custom",
            Stage::Path => "Path",
            Stage::Uvx => "uvx",
        }
    }
}

/// What one step found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StageResult {
    pub stage: Stage,
    /// The tool's executable, or `uvx` itself for [`Stage::Uvx`].
    pub found: Option<PathBuf>,
}

/// The result of a lookup, with every step of the chain (as the
/// reference's Lookup cell marks them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub tool: Tool,
    pub lookup: Lookup,
    pub stages: Vec<StageResult>,
    /// What runs the tool; `None` when nothing was found, or only `uvx`
    /// without permission to use it.
    pub executable: Option<Executable>,
    /// Only `uvx` could run the tool, and it may not without asking: the
    /// Language Services widget asks the person (ADR 0010).
    pub needs_uvx_consent: bool,
}

impl Resolution {
    /// The steps with marks, as the reference's Lookup cell shows them:
    /// `Sdk ✗ · Path ✓ · uvx ✓`.
    pub fn summary(&self) -> String {
        self.stages
            .iter()
            .map(|s| {
                let mark = if s.found.is_some() { '✓' } else { '✗' };
                format!("{} {mark}", s.stage.label())
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Looks a tool up by its lookup mode, going through every step.
pub fn resolve(tool: Tool, lookup: Lookup, context: &LookupContext) -> Resolution {
    let from_env;
    let search = match context.search {
        Some(search) => search,
        None => {
            from_env = SearchPaths::from_env();
            &from_env
        }
    };
    let name = tool.executable();
    let mut stages = Vec::new();
    if lookup == Lookup::Sdk {
        let found = context
            .interpreter
            .and_then(|python| py_env::paths::env_tool(python, name));
        stages.push(StageResult {
            stage: Stage::Sdk,
            found,
        });
    }
    if lookup != Lookup::Uvx {
        if let Some(custom) = context.custom_path {
            stages.push(StageResult {
                stage: Stage::Custom,
                found: is_executable(custom).then(|| custom.to_owned()),
            });
        }
        stages.push(StageResult {
            stage: Stage::Path,
            found: search.find(name),
        });
    }
    let uvx = search.find("uvx");
    stages.push(StageResult {
        stage: Stage::Uvx,
        found: uvx.clone(),
    });

    let local = stages
        .iter()
        .filter(|s| s.stage != Stage::Uvx)
        .find_map(|s| s.found.clone());
    let (executable, needs_uvx_consent) = match (local, uvx) {
        (Some(path), _) => (Some(Executable::Path(path)), false),
        (None, Some(uvx)) if context.allow_uvx => (Some(Executable::uvx(tool, uvx)), false),
        (None, Some(_)) => (None, true),
        (None, None) => (None, false),
    };
    Resolution {
        tool,
        lookup,
        stages,
        executable,
        needs_uvx_consent,
    }
}

/// Finds a tool's executable by its lookup mode (blocking: run it off the
/// UI thread). `None` when nothing is found and `uvx` is not allowed or
/// not installed.
pub fn find_executable(tool: Tool, lookup: Lookup, context: &LookupContext) -> Option<Executable> {
    resolve(tool, lookup, context).executable
}

// ---- versions --------------------------------------------------------------

/// How long a tool may take to answer `--version`.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// The most output of `--version` that is read.
const VERSION_OUTPUT_LIMIT: usize = 64 * 1024;

/// The version a tool reports for `--version` (`ruff 0.15.20`), from its
/// output; `None` when the output is not the tool's. As the reference
/// (`PyToolVersionCheck.parseVersion`): the first non-blank line that is
/// the package name, an optional comma and `(version`, and the version.
pub fn parse_version(tool: Tool, output: &str) -> Option<String> {
    let pattern = format!(
        r"(?i)^{},?(?:\s\(?version)?\s([^\s)]+).*$",
        regex::escape(tool.package())
    );
    let re = regex::Regex::new(&pattern).expect("the version pattern is valid");
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(|line| re.captures(line).map(|c| c[1].to_owned()))
}

/// The command that asks a tool for its version: `--version` of
/// [`Tool::version_executable`], the server's own file for ruff and ty,
/// the command line tool next to basedpyright's language server (or the
/// same through `uvx`). `None` when that tool is not there.
pub fn version_command(tool: Tool, executable: &Executable) -> Option<CommandLine> {
    match executable {
        Executable::Path(path) => {
            let program = if tool.version_executable() == tool.executable() {
                path.clone()
            } else {
                sibling(path, tool.executable(), tool.version_executable())?
            };
            Some(CommandLine::new(program).arg("--version"))
        }
        Executable::Uvx { uvx, package, .. } => Some(
            Executable::Uvx {
                uvx: uvx.clone(),
                package: package.clone(),
                executable: tool.version_executable().to_owned(),
            }
            .command_line(&["--version"]),
        ),
    }
}

/// The executable `name` next to `path`, with the extension `path` has
/// (`basedpyright.exe` next to `basedpyright-langserver.exe`).
fn sibling(path: &Path, own: &str, name: &str) -> Option<PathBuf> {
    let file = path.file_name()?.to_str()?;
    let suffix = file.strip_prefix(own).unwrap_or_default();
    let candidate = path.parent()?.join(format!("{name}{suffix}"));
    is_executable(&candidate).then_some(candidate)
}

/// Why a tool's version is not known.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    /// No command answers `--version` (basedpyright's command line tool is
    /// not next to its language server).
    #[error("no command reports the version")]
    NoCommand,
    /// It could not be started, or it failed.
    #[error("{0}")]
    Failed(String),
    #[error("no answer within {} s", .0.as_secs())]
    TimedOut(Duration),
    /// It is another program, as the reference says ("The selected tool is
    /// not {0}, it is {1}").
    #[error("The selected tool is not {expected}, it is {output}")]
    WrongTool { expected: String, output: String },
}

/// Runs `--version` of a tool (blocking, at most `timeout`) and reads the
/// version. Through `uvx` this may download the package: callers ask only
/// when `uvx` may run.
pub fn probe_version(
    tool: Tool,
    executable: &Executable,
    timeout: Duration,
) -> Result<String, VersionError> {
    let command = version_command(tool, executable).ok_or(VersionError::NoCommand)?;
    let (stdout, stderr, success) = run_for_output(&command, timeout, VERSION_OUTPUT_LIMIT)?;
    if let Some(version) = parse_version(tool, &stdout) {
        return Ok(version);
    }
    if !success {
        let text = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        return Err(VersionError::Failed(shorten(text.trim())));
    }
    Err(VersionError::WrongTool {
        expected: tool.package().to_owned(),
        output: shorten(stdout.trim()),
    })
}

/// The standard output, standard error and success of a short command,
/// each cut at `limit` bytes.
pub(crate) fn run_for_output(
    command: &CommandLine,
    timeout: Duration,
    limit: usize,
) -> Result<(String, String, bool), VersionError> {
    let (handle, events) =
        process::spawn(command).map_err(|e| VersionError::Failed(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let keep = |into: &mut Vec<u8>, chunk: Vec<u8>| {
        let room = limit.saturating_sub(into.len());
        into.extend_from_slice(&chunk[..chunk.len().min(room)]);
    };
    let success = loop {
        match events.recv_deadline(deadline) {
            Ok(ProcessEvent::Stdout(chunk)) => keep(&mut stdout, chunk),
            Ok(ProcessEvent::Stderr(chunk)) => keep(&mut stderr, chunk),
            Ok(ProcessEvent::Exited(status)) => break status.success(),
            Err(_) => {
                let _ = handle.kill();
                return Err(VersionError::TimedOut(timeout));
            }
        }
    };
    Ok((
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        success,
    ))
}

/// At most 250 characters, as the reference shortens the output it shows.
fn shorten(text: &str) -> String {
    const MAX: usize = 250;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

/// `version` is older than the oldest one Pyoxide is tested with
/// ([`Tool::minimum_version`]). Versions that do not parse are not judged.
pub fn below_minimum(tool: Tool, version: &str) -> bool {
    let minimum = tool.minimum_version().and_then(Version::parse);
    match (minimum, Version::parse(version)) {
        (Some(minimum), Some(version)) => version < minimum,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::MINIMUM_TY_VERSION;

    /// An executable file (on Unix: mode 755) with `text`.
    fn tool_file(dir: &Path, name: &str, text: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// The name of an executable on this OS (`ruff.exe` on Windows).
    fn exe(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    }

    /// Folders of a fake machine: a project interpreter, `PATH`, a
    /// per-user folder.
    struct Machine {
        _dir: tempfile::TempDir,
        python: PathBuf,
        venv_bin: PathBuf,
        path_dir: PathBuf,
        user_dir: PathBuf,
        search: SearchPaths,
    }

    impl Machine {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let python = py_env::paths::venv_python(&dir.path().join(".venv"));
            let venv_bin = python.parent().unwrap().to_path_buf();
            tool_file(&venv_bin, python.file_name().unwrap().to_str().unwrap(), "");
            let path_dir = dir.path().join("path");
            let user_dir = dir.path().join("home").join(".local").join("bin");
            std::fs::create_dir_all(&path_dir).unwrap();
            let search = SearchPaths {
                path: vec![dir.path().join("missing"), path_dir.clone()],
                user_dirs: vec![user_dir.clone()],
                extensions: if cfg!(windows) {
                    path_extensions(None)
                } else {
                    Vec::new()
                },
            };
            Self {
                _dir: dir,
                python,
                venv_bin,
                path_dir,
                user_dir,
                search,
            }
        }

        fn context<'a>(&'a self, custom: Option<&'a Path>, allow_uvx: bool) -> LookupContext<'a> {
            LookupContext {
                interpreter: Some(&self.python),
                custom_path: custom,
                allow_uvx,
                search: Some(&self.search),
            }
        }
    }

    #[test]
    fn modes_and_command_lines() {
        for mode in [Lookup::Sdk, Lookup::Path, Lookup::Uvx] {
            assert_eq!(Lookup::parse(mode.id()), Some(mode));
        }
        assert_eq!(Lookup::Uvx.label(), "uvx");
        let path = Path::new("venv").join("bin").join("ruff");
        let ruff = Executable::Path(path.clone()).command_line(Tool::Ruff.server_args());
        assert_eq!((ruff.program, ruff.args), (path, vec!["server".into()]));
        let uvx = |package: &str, executable: &str| Executable::Uvx {
            uvx: "uvx".into(),
            package: package.into(),
            executable: executable.into(),
        };
        assert_eq!(
            uvx("ty", "ty").command_line(&["server"]).args,
            ["ty", "server"]
        );
        assert_eq!(
            Executable::uvx(Tool::Basedpyright, "uvx".into())
                .command_line(&["--stdio"])
                .args,
            [
                "--from",
                "basedpyright",
                "basedpyright-langserver",
                "--stdio"
            ]
        );
        assert!(uvx("ty", "ty").is_uvx());
    }

    #[test]
    fn sdk_first_then_custom_then_path_then_user_folders() {
        let m = Machine::new();
        let ruff = exe("ruff");
        let custom = m.path_dir.join("..").join("custom").join(&ruff);
        let in_user_dir = tool_file(&m.user_dir, &ruff, "");
        let r = resolve(Tool::Ruff, Lookup::Sdk, &m.context(Some(&custom), false));
        // The custom path does not exist: passed over.
        assert_eq!(r.executable, Some(Executable::Path(in_user_dir)));
        assert_eq!(r.summary(), "Sdk ✗ · Custom ✗ · Path ✓ · uvx ✗");

        let on_path = tool_file(&m.path_dir, &ruff, "");
        let r = resolve(Tool::Ruff, Lookup::Sdk, &m.context(None, false));
        assert_eq!(r.executable, Some(Executable::Path(on_path.clone())));
        assert_eq!(r.summary(), "Sdk ✗ · Path ✓ · uvx ✗");

        let custom = tool_file(&m.path_dir.join("..").join("custom"), &ruff, "");
        let r = resolve(Tool::Ruff, Lookup::Sdk, &m.context(Some(&custom), false));
        assert_eq!(r.executable, Some(Executable::Path(custom.clone())));

        let in_venv = tool_file(&m.venv_bin, &ruff, "");
        let r = resolve(Tool::Ruff, Lookup::Sdk, &m.context(Some(&custom), false));
        assert_eq!(r.executable, Some(Executable::Path(in_venv)));
        assert_eq!(r.summary(), "Sdk ✓ · Custom ✓ · Path ✓ · uvx ✗");
        assert!(!r.needs_uvx_consent);

        // Path skips the interpreter.
        let r = resolve(Tool::Ruff, Lookup::Path, &m.context(None, false));
        assert_eq!(r.executable, Some(Executable::Path(on_path)));
        assert_eq!(r.summary(), "Path ✓ · uvx ✗");
        let r = resolve(Tool::Ruff, Lookup::Path, &m.context(Some(&custom), false));
        assert_eq!(r.executable, Some(Executable::Path(custom)));

        // No interpreter: the Sdk step finds nothing.
        let mut context = m.context(None, false);
        context.interpreter = None;
        let r = resolve(Tool::Ruff, Lookup::Sdk, &context);
        assert_eq!(r.stages[0].found, None);
        assert!(r.executable.is_some());
    }

    #[test]
    fn uvx_ends_the_chain_only_when_allowed() {
        let m = Machine::new();
        let r = resolve(Tool::Ty, Lookup::Sdk, &m.context(None, true));
        assert_eq!(r.executable, None);
        assert!(!r.needs_uvx_consent, "no uvx: nothing to ask");

        let uvx = tool_file(&m.user_dir, &exe("uvx"), "");
        let r = resolve(Tool::Ty, Lookup::Sdk, &m.context(None, false));
        assert_eq!(r.executable, None);
        assert!(r.needs_uvx_consent);
        assert_eq!(r.summary(), "Sdk ✗ · Path ✗ · uvx ✓");

        let r = resolve(Tool::Ty, Lookup::Sdk, &m.context(None, true));
        assert_eq!(r.executable, Some(Executable::uvx(Tool::Ty, uvx.clone())));
        assert!(!r.needs_uvx_consent);
        assert_eq!(
            find_executable(Tool::Basedpyright, Lookup::Path, &m.context(None, true))
                .unwrap()
                .command_line(Tool::Basedpyright.server_args())
                .display(),
            CommandLine::new(&uvx)
                .args([
                    "--from",
                    "basedpyright",
                    "basedpyright-langserver",
                    "--stdio"
                ])
                .display()
        );

        // uvx mode: always uvx, even with the tool installed.
        tool_file(&m.venv_bin, &exe("ty"), "");
        tool_file(&m.path_dir, &exe("ty"), "");
        let r = resolve(Tool::Ty, Lookup::Uvx, &m.context(None, true));
        assert_eq!(r.executable, Some(Executable::uvx(Tool::Ty, uvx)));
        assert_eq!(r.summary(), "uvx ✓");
        let r = resolve(Tool::Ty, Lookup::Sdk, &m.context(None, true));
        assert!(matches!(r.executable, Some(Executable::Path(_))));
    }

    #[test]
    fn path_extensions_are_tried_in_order() {
        // Windows' rules on any OS: names with the `PATHEXT` extensions,
        // never the bare name.
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        tool_file(&first, "ruff", "");
        let cmd = tool_file(&second, "basedpyright-langserver.cmd", "");
        let ruff = tool_file(&second, "ruff.exe", "");
        let search = SearchPaths {
            path: vec![first, second],
            user_dirs: Vec::new(),
            extensions: path_extensions(Some(".COM;.EXE; .Bat;;.CMD")),
        };
        assert_eq!(search.extensions, [".com", ".exe", ".bat", ".cmd"]);
        assert_eq!(search.find("ruff"), Some(ruff.clone()));
        assert_eq!(search.find("ruff.exe"), Some(ruff));
        assert_eq!(search.find("basedpyright-langserver"), Some(cmd));
        assert_eq!(search.find("ty"), None);
        assert_eq!(path_extensions(None), [".com", ".exe", ".bat", ".cmd"]);
        assert_eq!(path_extensions(Some("  ")).len(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn files_that_cannot_run_are_not_tools() {
        use std::os::unix::fs::PermissionsExt;
        let m = Machine::new();
        let ruff = tool_file(&m.path_dir, "ruff", "");
        std::fs::set_permissions(&ruff, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::create_dir_all(m.user_dir.join("ruff")).unwrap();
        assert_eq!(m.search.find("ruff"), None);
        let r = resolve(Tool::Ruff, Lookup::Path, &m.context(Some(&ruff), false));
        assert_eq!(r.executable, None);
    }

    #[cfg(windows)]
    #[test]
    fn sdk_lookup_in_scripts() {
        let m = Machine::new();
        assert!(m.venv_bin.ends_with("Scripts"));
        let ruff = tool_file(&m.venv_bin, "ruff.exe", "");
        let r = resolve(Tool::Ruff, Lookup::Sdk, &m.context(None, false));
        assert_eq!(r.executable, Some(Executable::Path(ruff)));
        let shim = tool_file(&m.venv_bin, "basedpyright-langserver.cmd", "");
        let r = resolve(Tool::Basedpyright, Lookup::Sdk, &m.context(None, false));
        assert_eq!(r.executable, Some(Executable::Path(shim)));
    }

    #[test]
    fn versions_as_the_reference_reads_them() {
        assert_eq!(
            parse_version(Tool::Ruff, "ruff 0.15.20\n").as_deref(),
            Some("0.15.20")
        );
        assert_eq!(
            parse_version(Tool::Ty, "ty 0.0.86 (2c3d4e5f6 2026-10-09)").as_deref(),
            Some("0.0.86")
        );
        assert_eq!(
            parse_version(
                Tool::Basedpyright,
                "basedpyright 1.39.9\nbased on pyright 1.1.411\n"
            )
            .as_deref(),
            Some("1.39.9")
        );
        // Warnings before the banner, other spellings.
        assert_eq!(
            parse_version(
                Tool::Basedpyright,
                "\n  WARNING: a newer version exists\nbasedpyright 1.40.2"
            )
            .as_deref(),
            Some("1.40.2")
        );
        assert_eq!(
            parse_version(Tool::Ruff, "Ruff, version 0.4.1").as_deref(),
            Some("0.4.1")
        );
        assert_eq!(
            parse_version(Tool::Ruff, "ruff (version 0.4.1)").as_deref(),
            Some("0.4.1")
        );
        assert_eq!(parse_version(Tool::Ty, "ruff 0.15.20"), None);
        assert_eq!(parse_version(Tool::Ty, "tyrant 1.0"), None);
        assert_eq!(parse_version(Tool::Ty, ""), None);

        assert!(below_minimum(Tool::Ty, "0.0.85"));
        assert!(below_minimum(Tool::Ty, "0.0.1a20"));
        assert!(!below_minimum(Tool::Ty, MINIMUM_TY_VERSION));
        assert!(!below_minimum(Tool::Ty, "0.1.0"));
        assert!(!below_minimum(Tool::Ty, "unknown"));
        assert!(!below_minimum(Tool::Ruff, "0.0.1"));
    }

    #[test]
    fn version_commands() {
        let dir = tempfile::tempdir().unwrap();
        let ty = Executable::Path(dir.path().join("ty"));
        let command = version_command(Tool::Ty, &ty).unwrap();
        assert_eq!(command.args, ["--version"]);
        assert_eq!(command.program, dir.path().join("ty"));

        let langserver = tool_file(dir.path(), "basedpyright-langserver.cmd", "");
        let server = Executable::Path(langserver);
        assert_eq!(version_command(Tool::Basedpyright, &server), None);
        let cli = tool_file(dir.path(), "basedpyright.cmd", "");
        assert_eq!(
            version_command(Tool::Basedpyright, &server)
                .unwrap()
                .program,
            cli
        );
        let uvx = Executable::uvx(Tool::Basedpyright, "uvx".into());
        assert_eq!(
            version_command(Tool::Basedpyright, &uvx).unwrap().args,
            ["basedpyright", "--version"]
        );
        assert_eq!(
            version_command(Tool::Ty, &Executable::uvx(Tool::Ty, "uvx".into()))
                .unwrap()
                .args,
            ["ty", "--version"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn probing_fake_tools() {
        let dir = tempfile::tempdir().unwrap();
        let script = |name: &str, body: &str| {
            Executable::Path(tool_file(dir.path(), name, &format!("#!/bin/sh\n{body}\n")))
        };
        let ty = script("ty", "echo 'ty 0.0.90 (abcdef 2026-10-12)'");
        assert_eq!(
            probe_version(Tool::Ty, &ty, VERSION_TIMEOUT).as_deref(),
            Ok("0.0.90")
        );

        let other = script("not-ty", "echo 'ruff 0.15.20'");
        assert_eq!(
            probe_version(Tool::Ty, &other, VERSION_TIMEOUT),
            Err(VersionError::WrongTool {
                expected: "ty".into(),
                output: "ruff 0.15.20".into()
            })
        );
        assert_eq!(
            VersionError::WrongTool {
                expected: "ty".into(),
                output: "ruff 0.15.20".into()
            }
            .to_string(),
            "The selected tool is not ty, it is ruff 0.15.20"
        );

        let failing = script("failing", "echo 'cannot run' >&2; exit 3");
        assert_eq!(
            probe_version(Tool::Ty, &failing, VERSION_TIMEOUT),
            Err(VersionError::Failed("cannot run".into()))
        );

        let slow = script("slow", "sleep 5");
        assert_eq!(
            probe_version(Tool::Ty, &slow, Duration::from_millis(200)),
            Err(VersionError::TimedOut(Duration::from_millis(200)))
        );

        let server = script(
            "basedpyright-langserver",
            "echo 'needs --stdio' >&2; exit 1",
        );
        assert_eq!(
            probe_version(Tool::Basedpyright, &server, VERSION_TIMEOUT),
            Err(VersionError::NoCommand)
        );
        script(
            "basedpyright",
            "echo 'basedpyright 1.40.2'; echo 'based on pyright 1.1.414'",
        );
        assert_eq!(
            probe_version(Tool::Basedpyright, &server, VERSION_TIMEOUT).as_deref(),
            Ok("1.40.2")
        );

        let missing = Executable::Path(dir.path().join("nothing-here"));
        assert!(matches!(
            probe_version(Tool::Ruff, &missing, VERSION_TIMEOUT),
            Err(VersionError::Failed(_))
        ));
        assert_eq!(shorten(&"x".repeat(300)).chars().count(), 250);
    }
}
