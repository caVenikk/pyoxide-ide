//! Whether a shell has a command running, which a terminal tab asks before
//! it closes (the reference's `TerminalUtil.hasRunningCommands`): the shell
//! has a child process.

/// The question for one shell, detached from its [`crate::Session`] so that
/// it can be asked on a background thread.
#[derive(Debug, Clone)]
pub struct CommandProbe {
    pid: u32,
    /// The shell is a launcher that starts the real shell as its only
    /// child (Git for Windows' `bin\bash.exe`).
    launcher: bool,
}

impl CommandProbe {
    pub(crate) fn new(pid: u32, program: &str) -> Self {
        Self {
            pid,
            launcher: cfg!(windows) && names_launcher(program),
        }
    }

    /// Whether a command runs in the shell: whether the shell has a child
    /// process. Reads the process table (a few milliseconds): call it off the
    /// UI thread. `false` for a shell that ended.
    pub fn has_running_commands(&self) -> bool {
        let mut shell = self.pid;
        if self.launcher
            && let [inner] = process::child_processes(shell)[..]
        {
            shell = inner;
        }
        process::has_child_processes(shell)
    }
}

/// Git for Windows' `bin\bash.exe` runs `usr\bin\bash.exe` as its child, so
/// a shell with nothing running has one child already (the reference counts
/// one more console process for it).
fn names_launcher(program: &str) -> bool {
    program
        .replace('\\', "/")
        .to_ascii_lowercase()
        .ends_with("/git/bin/bash.exe")
}

#[cfg(test)]
mod tests {
    use super::names_launcher;

    #[test]
    fn only_the_git_bash_launcher_is_one() {
        assert!(names_launcher("C:\\Program Files\\Git\\bin\\bash.exe"));
        assert!(names_launcher(
            "C:/Users/me/AppData/Local/Programs/Git/bin/bash.exe"
        ));
        assert!(!names_launcher(
            "C:\\Program Files\\Git\\usr\\bin\\bash.exe"
        ));
        assert!(!names_launcher("/bin/bash"));
    }
}
