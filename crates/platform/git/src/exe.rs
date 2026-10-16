use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A version of git: `git version 2.39.5 (Apple Git-154)` is 2.39.5,
/// `git version 2.41.0.windows.1` is 2.41.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// The oldest git the application works with (the reference's
    /// minimum): 2.17, and 2.19.2 on Windows.
    pub const MIN: Version = if cfg!(windows) {
        Version(2, 19, 2)
    } else {
        Version(2, 17, 0)
    };

    /// The version in the output of `git version`.
    pub fn parse(output: &str) -> Option<Self> {
        let rest = output.trim().strip_prefix("git version ")?;
        let number = rest.split_whitespace().next()?;
        let mut parts = number.split('.').map(|p| {
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>().ok()
        });
        let major = parts.next()??;
        let minor = parts.next().flatten().unwrap_or(0);
        let patch = parts.next().flatten().unwrap_or(0);
        Some(Self(major, minor, patch))
    }

    pub fn is_supported(self) -> bool {
        self >= Self::MIN
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Why no git can be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LookupError {
    /// No git executable in the settings, on `PATH` or in the usual
    /// folders.
    #[error("Git is not installed")]
    NotFound,
    /// The executable did not answer `git version`.
    #[error("Cannot run {path}: {message}")]
    NotRunnable { path: PathBuf, message: String },
    #[error(
        "Unsupported Git version {version}; the minimal supported version is {}",
        Version::MIN
    )]
    Unsupported { path: PathBuf, version: Version },
}

/// A git executable with its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitExe {
    pub path: PathBuf,
    pub version: Version,
}

const EXE: &str = if cfg!(windows) { "git.exe" } else { "git" };

/// Folders looked at after `PATH`: applications started from the macOS
/// Finder or the Dock do not get the shell's `PATH`.
#[cfg(unix)]
const UNIX_FOLDERS: &[&str] = &[
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/opt/local/bin",
    "/usr/bin",
    "/opt/bin",
    "/usr/local/git/bin",
];

impl GitExe {
    /// Finds git: the configured path (the settings' `[git] path`), else
    /// `PATH`, then the usual install folders; checks its version.
    pub fn find(configured: Option<&Path>) -> Result<Self, LookupError> {
        if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
            return Self::at(path);
        }
        let mut last_error = None;
        for candidate in candidates(std::env::var_os("PATH").as_deref()) {
            if !usable(&candidate) {
                continue;
            }
            match Self::at(&candidate) {
                Ok(exe) => return Ok(exe),
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error.unwrap_or(LookupError::NotFound))
    }

    /// The executable at `path`, when it runs and is new enough.
    pub fn at(path: &Path) -> Result<Self, LookupError> {
        // The first run of the macOS system git after an update of the
        // command line tools can fail; the reference tries again.
        let attempts = if is_macos_system_git(path) { 3 } else { 1 };
        let mut message = String::new();
        for _ in 0..attempts {
            match run_version(path) {
                Ok(text) => {
                    let Some(version) = Version::parse(&text) else {
                        message = format!("unexpected output: {}", text.trim());
                        continue;
                    };
                    if !version.is_supported() {
                        return Err(LookupError::Unsupported {
                            path: path.to_owned(),
                            version,
                        });
                    }
                    return Ok(Self {
                        path: path.to_owned(),
                        version,
                    });
                }
                Err(e) => message = e,
            }
        }
        Err(LookupError::NotRunnable {
            path: path.to_owned(),
            message,
        })
    }

    /// A command running this git with the options of every call.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.path);
        prepare(&mut cmd);
        cmd
    }
}

/// What every git call gets: no terminal prompts, machine-readable paths,
/// no signature lines in logs, messages in English (the reference's
/// `VcsLocaleHelper`: `LANGUAGE` empty, `LC_ALL=en_US.UTF-8`; a missing
/// locale falls back to git's own English), and no console window on
/// Windows.
pub(crate) fn prepare(cmd: &mut Command) {
    cmd.args([
        "-c",
        "core.quotepath=false",
        "-c",
        "log.showSignature=false",
    ])
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("LANGUAGE", "")
    .env("LC_ALL", "en_US.UTF-8")
    .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn run_version(path: &Path) -> Result<String, String> {
    let mut cmd = Command::new(path);
    prepare(&mut cmd);
    let out = cmd.arg("version").output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The places to look for git, in order, without duplicates.
fn candidates(path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(var) = path_var {
        out.extend(std::env::split_paths(var).map(|dir| dir.join(EXE)));
    }
    #[cfg(unix)]
    out.extend(UNIX_FOLDERS.iter().map(|d| Path::new(d).join(EXE)));
    #[cfg(windows)]
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            out.push(Path::new(&dir).join("Git").join("cmd").join(EXE));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// The candidate exists and running it does not ask to install anything.
fn usable(path: &Path) -> bool {
    path.is_file() && (!is_macos_system_git(path) || has_developer_tools())
}

/// `/usr/bin/git` on macOS is a shim that opens an install dialog when
/// neither the Command Line Tools nor Xcode are installed.
fn is_macos_system_git(path: &Path) -> bool {
    cfg!(target_os = "macos") && path == Path::new("/usr/bin/git")
}

/// The Command Line Tools or Xcode are installed: `xcode-select -p` names
/// a developer folder that has git.
fn has_developer_tools() -> bool {
    let Ok(out) = Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    dir.join("usr/bin/git").is_file()
        || dir
            .join("Toolchains/XcodeDefault.xctoolchain/usr/bin/git")
            .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_the_version_output() {
        assert_eq!(
            Version::parse("git version 2.39.5 (Apple Git-154)\n"),
            Some(Version(2, 39, 5))
        );
        assert_eq!(
            Version::parse("git version 2.41.0.windows.1"),
            Some(Version(2, 41, 0))
        );
        assert_eq!(
            Version::parse("git version 2.50.0-rc1"),
            Some(Version(2, 50, 0))
        );
        assert_eq!(Version::parse("git version 3"), Some(Version(3, 0, 0)));
        assert_eq!(Version::parse("hg version 6"), None);
        assert!(Version(2, 17, 0) >= Version(2, 16, 9));
        assert!(!Version(2, 16, 0).is_supported());
        assert!(Version(2, 30, 0).is_supported());
    }

    #[test]
    fn path_comes_first_then_the_usual_folders() {
        let dir = std::env::temp_dir();
        let path = std::env::join_paths([&dir]).unwrap();
        let list = candidates(Some(&path));
        assert_eq!(list[0], dir.join(EXE));
        #[cfg(unix)]
        assert!(list.contains(&PathBuf::from("/opt/homebrew/bin/git")));
    }

    #[test]
    fn the_installed_git_is_found() {
        let exe = GitExe::find(None).expect("git is installed on the test machines");
        assert!(exe.version.is_supported());
        assert_eq!(GitExe::at(&exe.path).unwrap(), exe);
    }

    #[test]
    fn a_missing_configured_git_is_an_error() {
        let missing = std::env::temp_dir().join("no-such-folder").join(EXE);
        assert!(matches!(
            GitExe::find(Some(&missing)),
            Err(LookupError::NotRunnable { .. })
        ));
    }
}
