//! Pyoxide's shell integration (ADR 0014): scripts for zsh, bash, fish and
//! PowerShell that read the user's startup files as the shell would, then
//! source the scripts customizers ask for (a virtual environment's
//! `activate`) before the first prompt, and report the working directory
//! (OSC 7) and the prompt and the commands with their exit codes (OSC 133
//! A, B, C, D) to the terminal.
//!
//! The scripts are embedded and written to a folder per version of their
//! text ([`prepare`]); [`inject`] changes a shell's command line and
//! environment so that it runs them, as the reference does
//! (`LocalShellIntegrationInjector`): zsh gets `ZDOTDIR`, bash `--rcfile`
//! (a login shell drops `--login`, which would skip it), fish
//! `--init-command` and PowerShell `-File`. `/bin/sh` of macOS (bash in
//! POSIX mode) ignores `--rcfile`, where the reference passes it: it gets
//! `ENV`, which an interactive sh reads. cmd.exe and other shells run
//! without integration.
//!
//! The scripts to source go to the shell as `__PYOXIDE_SOURCE_1`,
//! `__PYOXIDE_SOURCE_2`, ..., each `<script>\n<argument>\n...`.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::customizer::SourceScript;
use crate::start::{Os, ShellCommand, ShellKind};

/// The prefix of the variables naming the scripts to source
/// (`__PYOXIDE_SOURCE_1`, ...).
pub const SOURCE_VAR_PREFIX: &str = "__PYOXIDE_SOURCE_";
/// zsh: the folder of the integration's startup files.
pub const ZSH_DIR_VAR: &str = "__PYOXIDE_ZSH_DIR";
/// zsh: the `ZDOTDIR` the shell would have had.
pub const ORIGINAL_ZDOTDIR_VAR: &str = "__PYOXIDE_ORIGINAL_ZDOTDIR";
/// bash: the shell was asked to be a login shell.
pub const LOGIN_SHELL_VAR: &str = "__PYOXIDE_LOGIN_SHELL";
/// bash: the `--rcfile` the user's command line named.
pub const USER_RCFILE_VAR: &str = "__PYOXIDE_USER_RCFILE";
/// sh: the `ENV` the shell would have had.
pub const USER_ENV_VAR: &str = "__PYOXIDE_USER_ENV";

/// Variables the reference sets with its integration so that Fig and
/// Amazon Q (CodeWhisperer) do not start a nested shell without it.
const NESTED_SHELL_VARS: [(&str, &str); 3] = [
    ("FIG_TERM", "1"),
    ("PROCESS_LAUNCHED_BY_CW", "1"),
    ("PROCESS_LAUNCHED_BY_Q", "1"),
];

const ZSH_DIR: &str = "zsh";
const BASH_SCRIPT: &str = "bash/pyoxide-integration.bash";
const SH_SCRIPT: &str = "bash/pyoxide-integration.sh";
const FISH_SCRIPT: &str = "fish/pyoxide-integration.fish";
const POWERSHELL_SCRIPT: &str = "powershell/pyoxide-integration.ps1";
/// Written last: the folder is complete.
const COMPLETE: &str = ".complete";

/// An embedded script and its path in the integration folder.
struct Script {
    path: &'static str,
    text: &'static str,
}

const SCRIPTS: [Script; 10] = [
    Script {
        path: "zsh/.zshenv",
        text: include_str!("zsh/zshenv"),
    },
    Script {
        path: "zsh/.zprofile",
        text: include_str!("zsh/zprofile"),
    },
    Script {
        path: "zsh/.zshrc",
        text: include_str!("zsh/zshrc"),
    },
    Script {
        path: "zsh/.zlogin",
        text: include_str!("zsh/zlogin"),
    },
    Script {
        path: "zsh/source-original.zsh",
        text: include_str!("zsh/source-original.zsh"),
    },
    Script {
        path: "zsh/pyoxide-integration.zsh",
        text: include_str!("zsh/pyoxide-integration.zsh"),
    },
    Script {
        path: BASH_SCRIPT,
        text: include_str!("bash/pyoxide-integration.bash"),
    },
    Script {
        path: SH_SCRIPT,
        text: include_str!("bash/pyoxide-integration.sh"),
    },
    Script {
        path: FISH_SCRIPT,
        text: include_str!("fish/pyoxide-integration.fish"),
    },
    Script {
        path: POWERSHELL_SCRIPT,
        text: include_str!("powershell/pyoxide-integration.ps1"),
    },
];

/// A script's text with Unix line ends, whatever the checkout did (bash
/// and zsh do not read `\r\n`).
fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The version of the scripts: a hash of their paths and texts, so that a
/// changed script gets a folder of its own.
pub fn version() -> String {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for script in &SCRIPTS {
        add(script.path.as_bytes());
        add(&[0]);
        add(normalized(script.text).as_bytes());
        add(&[0]);
    }
    format!("{hash:016x}")
}

/// Where the integration folders are written by default:
/// `pyoxide/shell-integration` in the user's cache folder.
pub fn default_base_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("pyoxide").join("shell-integration"))
}

/// The folder of this version's scripts in `base`, written unless it is
/// complete already (each file replaced at once, so sessions starting in
/// other processes read whole files). Touches the file system: call it off
/// the UI thread.
pub fn prepare(base: &Path) -> io::Result<PathBuf> {
    let dir = base.join(version());
    if dir.join(COMPLETE).is_file() {
        return Ok(dir);
    }
    for script in &SCRIPTS {
        write_atomic(
            &script_path(&dir, script.path),
            normalized(script.text).as_bytes(),
        )?;
    }
    write_atomic(&dir.join(COMPLETE), b"")?;
    Ok(dir)
}

/// `dir` joined with a path written with `/`.
fn script_path(dir: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(dir.to_owned(), |p, part| p.join(part))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("no parent folder"))?;
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Whether Pyoxide has an integration for the shell: zsh, bash, fish,
/// PowerShell, and `sh` on macOS (bash there).
pub fn supports(kind: ShellKind, os: Os) -> bool {
    match kind {
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::PowerShell => true,
        ShellKind::Sh => os == Os::MacOs,
        ShellKind::Cmd | ShellKind::Other => false,
    }
}

/// A shell's command line and the variables to set for the integration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injection {
    pub command: ShellCommand,
    pub env: Vec<(String, String)>,
}

/// The command line and the variables that make `command` run the
/// integration of the folder `dir` ([`prepare`]) and source `sources`;
/// `None` for a shell without integration ([`supports`]).
/// `session_var` gives the variables the session would have without the
/// integration (`ZDOTDIR` for zsh, `ENV` for sh); `home` expands `~/` in a
/// `--rcfile` of the command line (bash).
pub fn inject(
    os: Os,
    dir: &Path,
    command: &ShellCommand,
    sources: &[SourceScript],
    session_var: &dyn Fn(&str) -> Option<String>,
    home: Option<&Path>,
) -> Option<Injection> {
    let kind = command.kind();
    if !supports(kind, os) {
        return None;
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let mut set = |name: &str, value: String| env.push((name.to_owned(), value));
    let mut args = command.args.clone();
    let mut first: Vec<String> = Vec::new();
    match kind {
        ShellKind::Bash => {
            let rcfile = path_text(&script_path(dir, BASH_SCRIPT));
            if drop_login(&mut args) {
                set(LOGIN_SHELL_VAR, "1".into());
            }
            if let Some(at) = args.iter().position(|a| a == "--rcfile") {
                args.remove(at);
                if at < args.len() {
                    let user = expand_home(&args.remove(at), home);
                    if user != rcfile {
                        set(USER_RCFILE_VAR, user);
                    }
                }
            }
            first = vec!["--rcfile".into(), rcfile];
        }
        ShellKind::Sh => {
            if drop_login(&mut args) {
                set(LOGIN_SHELL_VAR, "1".into());
            }
            if let Some(original) = session_var("ENV") {
                set(USER_ENV_VAR, original);
            }
            set("ENV", path_text(&script_path(dir, SH_SCRIPT)));
        }
        ShellKind::Zsh => {
            let zsh_dir = path_text(&dir.join(ZSH_DIR));
            if let Some(original) = session_var("ZDOTDIR").filter(|z| !z.is_empty()) {
                set(ORIGINAL_ZDOTDIR_VAR, original);
            }
            set("ZDOTDIR", zsh_dir.clone());
            set(ZSH_DIR_VAR, zsh_dir);
        }
        ShellKind::Fish => {
            // Several `--init-command`s are allowed (fish 2.7 and later).
            let script = path_text(&script_path(dir, FISH_SCRIPT));
            first = vec![format!("--init-command=source {}", fish_quote(&script))];
        }
        ShellKind::PowerShell => {
            // `-File` ends the options: the user's go first.
            let script = path_text(&script_path(dir, POWERSHELL_SCRIPT));
            args.extend(
                ["-NoExit", "-ExecutionPolicy", "Bypass", "-File"]
                    .into_iter()
                    .map(String::from)
                    .chain([script]),
            );
        }
        ShellKind::Cmd | ShellKind::Other => return None,
    }
    for (name, value) in NESTED_SHELL_VARS {
        set(name, value.into());
    }
    env.extend(source_vars(sources));
    first.extend(args);
    Some(Injection {
        command: ShellCommand {
            program: command.program.clone(),
            args: first,
        },
        env,
    })
}

/// Removes `--login` and `-l`, which would make bash read the profile files
/// instead of `--rcfile` (and sh before `ENV`): the scripts read them.
/// Whether there was one.
fn drop_login(args: &mut Vec<String>) -> bool {
    let before = args.len();
    args.retain(|a| a != "--login" && a != "-l");
    args.len() != before
}

/// `__PYOXIDE_SOURCE_<n>` for each script that can be passed this way (no
/// line break or NUL in its path or arguments, a UTF-8 path).
pub fn source_vars(sources: &[SourceScript]) -> Vec<(String, String)> {
    let passable = |s: &str| !s.contains(['\n', '\r', '\0']);
    sources
        .iter()
        .filter_map(|s| {
            let path = s.path.to_str().filter(|p| passable(p) && !p.is_empty())?;
            s.args.iter().all(|a| passable(a)).then(|| {
                std::iter::once(path)
                    .chain(s.args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .enumerate()
        .map(|(i, value)| (format!("{SOURCE_VAR_PREFIX}{}", i + 1), value))
        .collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `~/x` with the home folder.
fn expand_home(path: &str, home: Option<&Path>) -> String {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => path_text(&home.join(rest)),
        _ => path.to_owned(),
    }
}

/// A word in single quotes for fish (`\` and `'` escaped).
fn fish_quote(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str]) -> ShellCommand {
        ShellCommand {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn get<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn activate() -> Vec<SourceScript> {
        vec![SourceScript {
            path: PathBuf::from("/p/.venv/bin/activate"),
            args: Vec::new(),
        }]
    }

    #[test]
    fn scripts_are_written_once_per_version() {
        let base = tempfile::tempdir().unwrap();
        let dir = prepare(base.path()).unwrap();
        assert_eq!(dir, base.path().join(version()));
        assert_eq!(version().len(), 16);
        for script in &SCRIPTS {
            let text = std::fs::read_to_string(script_path(&dir, script.path)).unwrap();
            assert_eq!(text, normalized(script.text), "{}", script.path);
            assert!(!text.contains('\r'), "{}", script.path);
        }
        for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            assert!(dir.join("zsh").join(name).is_file(), "{name}");
        }
        // A complete folder is not written again.
        let zshrc = dir.join("zsh").join(".zshrc");
        std::fs::write(&zshrc, "changed").unwrap();
        assert_eq!(prepare(base.path()).unwrap(), dir);
        assert_eq!(std::fs::read_to_string(&zshrc).unwrap(), "changed");
        // An incomplete one is.
        std::fs::remove_file(dir.join(COMPLETE)).unwrap();
        prepare(base.path()).unwrap();
        assert_ne!(std::fs::read_to_string(&zshrc).unwrap(), "changed");
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("zsh"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn supported_shells() {
        for os in [Os::MacOs, Os::Linux, Os::Windows] {
            for kind in [
                ShellKind::Bash,
                ShellKind::Zsh,
                ShellKind::Fish,
                ShellKind::PowerShell,
            ] {
                assert!(supports(kind, os), "{kind:?} on {os:?}");
            }
            assert!(!supports(ShellKind::Cmd, os));
            assert!(!supports(ShellKind::Other, os));
        }
        assert!(supports(ShellKind::Sh, Os::MacOs));
        assert!(!supports(ShellKind::Sh, Os::Linux));
        let dir = Path::new("/cache/v1");
        let cmd = command("C:\\Windows\\System32\\cmd.exe", &[]);
        assert_eq!(
            inject(Os::Windows, dir, &cmd, &activate(), &none, None),
            None
        );
        let tcsh = command("/bin/tcsh", &[]);
        assert_eq!(inject(Os::Linux, dir, &tcsh, &[], &none, None), None);
    }

    #[test]
    fn zsh_gets_zdotdir() {
        let dir = Path::new("/cache/v1");
        let zsh_dir = path_text(&dir.join("zsh"));
        let cmd = command("/bin/zsh", &["--login", "-i"]);
        let zdotdir = |name: &str| (name == "ZDOTDIR").then(|| "/u/.config/zsh".to_owned());
        let out = inject(Os::MacOs, dir, &cmd, &activate(), &zdotdir, None).unwrap();
        assert_eq!(out.command, cmd);
        assert_eq!(get(&out.env, "ZDOTDIR"), Some(zsh_dir.as_str()));
        assert_eq!(get(&out.env, ZSH_DIR_VAR), Some(zsh_dir.as_str()));
        assert_eq!(get(&out.env, ORIGINAL_ZDOTDIR_VAR), Some("/u/.config/zsh"));
        assert_eq!(
            get(&out.env, "__PYOXIDE_SOURCE_1"),
            Some("/p/.venv/bin/activate")
        );
        assert_eq!(get(&out.env, "FIG_TERM"), Some("1"));
        assert_eq!(get(&out.env, "PROCESS_LAUNCHED_BY_Q"), Some("1"));
        // No ZDOTDIR before (or an empty one): none to restore.
        let empty = |name: &str| (name == "ZDOTDIR").then(String::new);
        let out = inject(Os::Linux, dir, &cmd, &[], &empty, None).unwrap();
        assert_eq!(get(&out.env, ORIGINAL_ZDOTDIR_VAR), None);
        assert_eq!(get(&out.env, "__PYOXIDE_SOURCE_1"), None);
    }

    #[test]
    fn bash_gets_rcfile_without_login() {
        let dir = Path::new("/cache/v1");
        let rcfile = path_text(&script_path(dir, BASH_SCRIPT));
        let cmd = command("/bin/bash", &["--login", "-i"]);
        let out = inject(Os::MacOs, dir, &cmd, &activate(), &none, None).unwrap();
        assert_eq!(
            out.command,
            command("/bin/bash", &["--rcfile", &rcfile, "-i"])
        );
        assert_eq!(get(&out.env, LOGIN_SHELL_VAR), Some("1"));
        assert_eq!(get(&out.env, USER_RCFILE_VAR), None);

        // Not a login shell; the user's own rcfile is read instead of
        // ~/.bashrc.
        let home = Path::new("/home/u");
        let cmd = command("/usr/bin/bash", &["--rcfile", "~/my.rc", "-i"]);
        let out = inject(Os::Linux, dir, &cmd, &[], &none, Some(home)).unwrap();
        assert_eq!(
            out.command,
            command("/usr/bin/bash", &["--rcfile", &rcfile, "-i"])
        );
        assert_eq!(get(&out.env, LOGIN_SHELL_VAR), None);
        let expected = path_text(&home.join("my.rc"));
        assert_eq!(get(&out.env, USER_RCFILE_VAR), Some(expected.as_str()));

        // `-l` counts as --login.
        let cmd = command("/bin/bash", &["-l", "-i"]);
        let out = inject(Os::MacOs, dir, &cmd, &[], &none, None).unwrap();
        assert_eq!(
            out.command,
            command("/bin/bash", &["--rcfile", &rcfile, "-i"])
        );
        assert_eq!(get(&out.env, LOGIN_SHELL_VAR), Some("1"));
        // Our own rcfile named again is not sourced twice.
        let cmd = command("bash", &["--rcfile", &rcfile]);
        let out = inject(Os::Linux, dir, &cmd, &[], &none, None).unwrap();
        assert_eq!(out.command, command("bash", &["--rcfile", &rcfile]));
        assert_eq!(get(&out.env, USER_RCFILE_VAR), None);
    }

    #[test]
    fn sh_on_macos_gets_env() {
        // /bin/sh of macOS is bash in POSIX mode: it ignores --rcfile and
        // reads $ENV, after the profile files of a login shell.
        let dir = Path::new("/cache/v1");
        let script = path_text(&script_path(dir, SH_SCRIPT));
        let cmd = command("/bin/sh", &["--login", "-i"]);
        let out = inject(Os::MacOs, dir, &cmd, &activate(), &none, None).unwrap();
        assert_eq!(out.command, command("/bin/sh", &["-i"]));
        assert_eq!(get(&out.env, "ENV"), Some(script.as_str()));
        assert_eq!(get(&out.env, LOGIN_SHELL_VAR), Some("1"));
        assert_eq!(get(&out.env, USER_ENV_VAR), None);
        assert_eq!(
            get(&out.env, "__PYOXIDE_SOURCE_1"),
            Some("/p/.venv/bin/activate")
        );
        // The user's ENV is read by the script; an empty one is kept as it
        // is.
        let user_env = |name: &str| (name == "ENV").then(|| "$HOME/.shrc".to_owned());
        let cmd = command("/bin/sh", &["-i"]);
        let out = inject(Os::MacOs, dir, &cmd, &[], &user_env, None).unwrap();
        assert_eq!(out.command, cmd);
        assert_eq!(get(&out.env, LOGIN_SHELL_VAR), None);
        assert_eq!(get(&out.env, USER_ENV_VAR), Some("$HOME/.shrc"));
        let empty = |name: &str| (name == "ENV").then(String::new);
        let out = inject(Os::MacOs, dir, &cmd, &[], &empty, None).unwrap();
        assert_eq!(get(&out.env, USER_ENV_VAR), Some(""));
        // sh elsewhere (dash and others) has no integration.
        assert_eq!(inject(Os::Linux, dir, &cmd, &[], &none, None), None);
    }

    #[test]
    fn fish_gets_an_init_command() {
        let dir = Path::new("/cache/it's");
        let script = path_text(&script_path(dir, FISH_SCRIPT));
        let cmd = command("/opt/homebrew/bin/fish", &["--login", "-i"]);
        let out = inject(Os::MacOs, dir, &cmd, &activate(), &none, None).unwrap();
        assert_eq!(out.command.program, cmd.program);
        assert_eq!(
            out.command.args[0],
            format!(
                "--init-command=source '{}'",
                script.replace('\\', "\\\\").replace('\'', "\\'")
            )
        );
        assert_eq!(out.command.args[1..], ["--login", "-i"]);
        assert_eq!(fish_quote(r"C:\a b\x'y"), r"'C:\\a b\\x\'y'");
    }

    #[test]
    fn powershell_gets_a_file_after_the_users_options() {
        let dir = Path::new("C:\\Cache\\v1");
        let script = path_text(&script_path(dir, POWERSHELL_SCRIPT));
        let cmd = command("powershell.exe", &["-NoLogo"]);
        let out = inject(Os::Windows, dir, &cmd, &activate(), &none, None).unwrap();
        assert_eq!(
            out.command,
            command(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoExit",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    &script
                ]
            )
        );
        assert_eq!(
            get(&out.env, "__PYOXIDE_SOURCE_1"),
            Some("/p/.venv/bin/activate")
        );
    }

    #[test]
    fn sources_with_arguments() {
        let sources = vec![
            SourceScript {
                path: PathBuf::from("/opt/conda/bin/activate"),
                args: vec!["/opt/conda/envs/my env".into()],
            },
            SourceScript {
                path: PathBuf::from("/bad\npath"),
                args: Vec::new(),
            },
            SourceScript {
                path: PathBuf::from("/x/y.sh"),
                args: vec!["".into(), "b".into()],
            },
        ];
        assert_eq!(
            source_vars(&sources),
            vec![
                (
                    "__PYOXIDE_SOURCE_1".to_owned(),
                    "/opt/conda/bin/activate\n/opt/conda/envs/my env".to_owned()
                ),
                ("__PYOXIDE_SOURCE_2".to_owned(), "/x/y.sh\n\nb".to_owned()),
            ]
        );
    }
}
