//! The shells offered in the New Predefined Session popup, found as the
//! reference finds them (`TerminalShellsDetectionUtil`): bash, zsh, fish
//! and pwsh in the standard folders on macOS and Linux; Windows
//! PowerShell, Command Prompt, PowerShell, Git Bash, Cmder and the WSL
//! distributions on Windows.

use std::path::{Path, PathBuf};

use crate::start::{Os, ShellCommand, shell_command_for};

/// Where a detected shell runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellEnvironment {
    Local,
    Wsl,
}

/// A shell found on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedShell {
    /// The name shown in the popup ("zsh", "Windows PowerShell", a WSL
    /// distribution name).
    pub name: String,
    /// The program and its options (`--login`, `-i` as the start rules add
    /// them, `/k init.bat` for Cmder, `-d <name>` for WSL).
    pub command: ShellCommand,
    pub environment: ShellEnvironment,
}

const UNIX_DIRECTORIES: [&str; 4] = ["/bin", "/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"];
const UNIX_SHELLS: [&str; 4] = ["bash", "zsh", "fish", "pwsh"];

/// The shells of this machine. Touches the file system (and the registry on
/// Windows): call it off the UI thread.
pub fn detect_shells() -> Vec<DetectedShell> {
    #[cfg(windows)]
    {
        let env = |name: &str| std::env::var(name).ok();
        let mut shells = detect_windows_shells(&env, &|p: &Path| p.is_file());
        shells.extend(windows::wsl_distributions().into_iter().map(wsl_shell));
        shells
    }
    #[cfg(not(windows))]
    {
        detect_unix_shells(Os::current(), &|p: &Path| p.is_file())
    }
}

/// Every existing `<dir>/<shell>` of the standard folders, folder by folder.
pub fn detect_unix_shells(os: Os, is_file: &dyn Fn(&Path) -> bool) -> Vec<DetectedShell> {
    let mut out = Vec::new();
    for dir in UNIX_DIRECTORIES {
        for name in UNIX_SHELLS {
            let path = format!("{dir}/{name}");
            if is_file(Path::new(&path)) {
                out.push(DetectedShell {
                    name: name.to_owned(),
                    command: shell_command_for(os, &path, |_| true),
                    environment: ShellEnvironment::Local,
                });
            }
        }
    }
    out
}

/// The local Windows shells, in the reference's order. `env` reads an
/// environment variable (`SystemRoot`, `ProgramFiles`, `LocalAppData`,
/// `CMDER_ROOT`, `PATH`, `PATHEXT`).
pub fn detect_windows_shells(
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Vec<DetectedShell> {
    let shell = |name: &str, path: &Path, extra: &[String]| {
        let mut command = shell_command_for(Os::Windows, &path.to_string_lossy(), |_| true);
        command.args.extend(extra.iter().cloned());
        DetectedShell {
            name: name.to_owned(),
            command,
            environment: ShellEnvironment::Local,
        }
    };
    let path_var = env("PATH").unwrap_or_default();
    let which = |exe: &str| find_on_path(exe, &path_var, is_file);
    let system_root = env("SystemRoot");
    let under = |path: &Path, prefix: &str| {
        path.to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    };
    let mut out = Vec::new();
    if let Some(root) = &system_root
        && let Some(ps) = which("powershell.exe")
        && under(&ps, &format!("{root}\\System32\\WindowsPowerShell\\"))
    {
        out.push(shell("Windows PowerShell", &ps, &[]));
    }
    let cmd = system_root
        .as_ref()
        .and_then(|root| which("cmd.exe").filter(|cmd| under(cmd, &format!("{root}\\System32\\"))));
    if let Some(cmd) = &cmd {
        out.push(shell("Command Prompt", cmd, &[]));
    }
    if let Some(pwsh) = which("pwsh.exe") {
        out.push(shell("PowerShell", &pwsh, &[]));
    }
    if let (Some(program_files), Some(local_app_data)) = (env("ProgramFiles"), env("LocalAppData"))
    {
        let local = PathBuf::from(format!("{local_app_data}\\Programs\\Git\\bin\\bash.exe"));
        let global = PathBuf::from(format!("{program_files}\\Git\\bin\\bash.exe"));
        if let Some(bash) = [local, global].into_iter().find(|p| is_file(p)) {
            out.push(shell("Git Bash", &bash, &[]));
        }
    }
    if let (Some(root), Some(cmd)) = (env("CMDER_ROOT"), &cmd) {
        let init = PathBuf::from(format!("{root}\\vendor\\init.bat"));
        if is_file(&init) {
            out.push(shell(
                "Cmder",
                cmd,
                &["/k".to_owned(), init.to_string_lossy().into_owned()],
            ));
        }
    }
    out
}

/// A WSL distribution as a shell: `wsl.exe -d <name>`.
pub fn wsl_shell(distribution: String) -> DetectedShell {
    DetectedShell {
        command: ShellCommand {
            program: "wsl.exe".into(),
            args: vec!["-d".into(), distribution.clone()],
        },
        name: distribution,
        environment: ShellEnvironment::Wsl,
    }
}

/// The first `<dir>\<exe>` on a Windows `PATH`.
fn find_on_path(exe: &str, path_var: &str, is_file: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    path_var
        .split(';')
        .map(|dir| dir.trim().trim_matches('"'))
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(format!("{}\\{exe}", dir.trim_end_matches('\\'))))
        .find(|p| is_file(p))
}

/// The labels of the popup: the name, or `name (path)` when several shells
/// share a name.
pub fn display_names(shells: &[DetectedShell]) -> Vec<String> {
    shells
        .iter()
        .map(|shell| {
            let shared = shells.iter().filter(|s| s.name == shell.name).count() > 1;
            if shared {
                format!("{} ({})", shell.name, shell.command.program)
            } else {
                shell.name.clone()
            }
        })
        .collect()
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
        RegOpenKeyExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The installed WSL distributions, from
    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss\{id}\DistributionName`.
    pub(super) fn wsl_distributions() -> Vec<String> {
        let mut out = Vec::new();
        let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Lxss");
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: valid, NUL-terminated strings and out pointers.
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) }
            != ERROR_SUCCESS
        {
            return out;
        }
        let value = wide("DistributionName");
        for index in 0.. {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            // SAFETY: `name` holds `name_len` characters.
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let mut sub = name[..name_len as usize].to_vec();
            sub.push(0);
            let mut data = [0u16; 512];
            let mut size = (data.len() * 2) as u32;
            // SAFETY: `data` holds `size` bytes.
            let status = unsafe {
                RegGetValueW(
                    key,
                    sub.as_ptr(),
                    value.as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    data.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if status == ERROR_SUCCESS {
                let len = (size as usize / 2).saturating_sub(1);
                let text = String::from_utf16_lossy(&data[..len.min(data.len())]);
                let text = text.trim_end_matches('\0').to_owned();
                if !text.is_empty() {
                    out.push(text);
                }
            }
        }
        // SAFETY: `key` was opened above.
        unsafe { RegCloseKey(key) };
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_shells_in_standard_folders() {
        let present = [
            "/bin/zsh",
            "/bin/bash",
            "/opt/homebrew/bin/fish",
            "/opt/homebrew/bin/bash",
        ];
        let is_file = |p: &Path| present.iter().any(|x| Path::new(x) == p);
        let shells = detect_unix_shells(Os::MacOs, &is_file);
        let found: Vec<(&str, &str)> = shells
            .iter()
            .map(|s| (s.name.as_str(), s.command.program.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("bash", "/bin/bash"),
                ("zsh", "/bin/zsh"),
                ("bash", "/opt/homebrew/bin/bash"),
                ("fish", "/opt/homebrew/bin/fish"),
            ]
        );
        assert_eq!(shells[1].command.args, ["--login", "-i"]);
        assert_eq!(
            display_names(&shells),
            [
                "bash (/bin/bash)",
                "zsh",
                "bash (/opt/homebrew/bin/bash)",
                "fish"
            ]
        );
        let linux = detect_unix_shells(Os::Linux, &is_file);
        assert_eq!(linux[0].command.args, ["-i"]);
    }

    #[test]
    fn windows_shells_in_reference_order() {
        let vars = [
            ("SystemRoot", "C:\\Windows"),
            ("ProgramFiles", "C:\\Program Files"),
            ("LocalAppData", "C:\\Users\\u\\AppData\\Local"),
            ("CMDER_ROOT", "C:\\cmder"),
            (
                "PATH",
                "C:\\Windows\\System32;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;\"C:\\Program Files\\PowerShell\\7\"",
            ),
        ];
        let env = |name: &str| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        };
        let files = [
            "C:\\Windows\\System32\\cmd.exe",
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
            "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
            "C:\\Program Files\\Git\\bin\\bash.exe",
            "C:\\cmder\\vendor\\init.bat",
        ];
        let is_file = |p: &Path| files.iter().any(|f| Path::new(f) == p);
        let shells = detect_windows_shells(&env, &is_file);
        let names: Vec<&str> = shells.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Windows PowerShell",
                "Command Prompt",
                "PowerShell",
                "Git Bash",
                "Cmder"
            ]
        );
        assert_eq!(
            shells[3].command.program,
            "C:\\Program Files\\Git\\bin\\bash.exe"
        );
        assert_eq!(
            shells[4].command.args,
            ["/k", "C:\\cmder\\vendor\\init.bat"]
        );
        let wsl = wsl_shell("Ubuntu".into());
        assert_eq!(wsl.command.args, ["-d", "Ubuntu"]);
        assert_eq!(wsl.environment, ShellEnvironment::Wsl);
    }
}
