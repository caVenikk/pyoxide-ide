/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExitStatus {
    /// The exit code, when the process exited by itself. On Windows a code
    /// above `i32::MAX` (an `NTSTATUS` such as `0xC000013A`) is negative.
    pub code: Option<i32>,
    /// The signal that terminated the process (Unix only).
    pub signal: Option<i32>,
}

impl ExitStatus {
    /// Exited with code 0.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// The exit code as the reference reports it: the code, or 128 plus the
    /// signal number for a process terminated by a signal (the JVM's
    /// convention, as shells do).
    pub fn exit_code(&self) -> i32 {
        match (self.code, self.signal) {
            (Some(code), _) => code,
            (None, Some(signal)) => 128 + signal,
            (None, None) => -1,
        }
    }

    /// [`ExitStatus::exit_code`] with the reference's explanation
    /// (`ProcessTerminatedListener.stringifyExitCode`): `130 (interrupted by
    /// signal 2:SIGINT)` on Unix, `-1073741510 (0xC000013A: interrupted by
    /// Ctrl+C)` on Windows, `0` otherwise.
    pub fn exit_code_text(&self) -> String {
        exit_code_text(self.exit_code(), Os::current())
    }

    /// The line the reference prints after the output, without line breaks:
    /// `Process finished with exit code 0`. The console shows it as a system
    /// line after an empty line: the reference prints
    /// `"\nProcess finished with exit code N\n"`.
    pub fn finished_message(&self) -> String {
        format!("Process finished with exit code {}", self.exit_code_text())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Os {
    Darwin,
    Linux,
    Windows,
}

impl Os {
    pub(crate) fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_vendor = "apple") {
            Os::Darwin
        } else {
            Os::Linux
        }
    }
}

/// The signals of the reference's `UnixSignal`: name, Darwin number, Linux
/// number.
const UNIX_SIGNALS: &[(&str, i32, i32)] = &[
    ("SIGHUP", 1, 1),
    ("SIGINT", 2, 2),
    ("SIGQUIT", 3, 3),
    ("SIGILL", 4, 4),
    ("SIGTRAP", 5, 5),
    ("SIGABRT", 6, 6),
    ("SIGBUS", 10, 7),
    ("SIGFPE", 8, 8),
    ("SIGKILL", 9, 9),
    ("SIGUSR1", 30, 10),
    ("SIGSEGV", 11, 11),
    ("SIGUSR2", 31, 12),
    ("SIGPIPE", 13, 13),
    ("SIGALRM", 14, 14),
    ("SIGTERM", 15, 15),
    ("SIGCHLD", 20, 17),
    ("SIGCONT", 19, 18),
    ("SIGSTOP", 17, 19),
    ("SIGTSTP", 18, 20),
    ("SIGTTIN", 21, 21),
    ("SIGTTOU", 22, 22),
    ("SIGURG", 16, 23),
    ("SIGXCPU", 24, 24),
    ("SIGXFSZ", 25, 25),
    ("SIGVTALRM", 26, 26),
    ("SIGPROF", 27, 27),
    ("SIGWINCH", 28, 28),
    ("SIGIO", 23, 29),
    ("SIGSYS", 12, 31),
];

pub(crate) fn exit_code_text(code: i32, os: Os) -> String {
    let mut out = code.to_string();
    match os {
        Os::Windows => {
            // 0xC0000000 <= code < 0xD0000000 as unsigned: an NTSTATUS error.
            let unsigned = code as u32;
            if (0xC000_0000..0xD000_0000).contains(&unsigned) {
                out.push_str(&format!(" (0x{unsigned:X}"));
                if unsigned == 0xC000_013A {
                    out.push_str(": interrupted by Ctrl+C");
                }
                out.push(')');
            }
        }
        Os::Darwin | Os::Linux => {
            let darwin = os == Os::Darwin;
            let found = UNIX_SIGNALS.iter().find(|(_, d, l)| {
                let number = if darwin { *d } else { *l };
                number + 128 == code
            });
            if let Some((name, d, l)) = found {
                let number = if darwin { d } else { l };
                out.push_str(&format!(" (interrupted by signal {number}:{name})"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_follows_the_jvm_convention() {
        let killed = ExitStatus {
            code: None,
            signal: Some(9),
        };
        assert_eq!(killed.exit_code(), 137);
        let ok = ExitStatus {
            code: Some(0),
            signal: None,
        };
        assert!(ok.success());
        assert_eq!(ok.exit_code(), 0);
    }

    #[test]
    fn exit_code_text_names_signals_and_ntstatus() {
        assert_eq!(exit_code_text(0, Os::Linux), "0");
        assert_eq!(exit_code_text(1, Os::Darwin), "1");
        assert_eq!(
            exit_code_text(130, Os::Darwin),
            "130 (interrupted by signal 2:SIGINT)"
        );
        assert_eq!(
            exit_code_text(137, Os::Linux),
            "137 (interrupted by signal 9:SIGKILL)"
        );
        // SIGUSR1 has different numbers on Darwin and Linux.
        assert_eq!(
            exit_code_text(158, Os::Darwin),
            "158 (interrupted by signal 30:SIGUSR1)"
        );
        assert_eq!(
            exit_code_text(138, Os::Linux),
            "138 (interrupted by signal 10:SIGUSR1)"
        );
        assert_eq!(
            exit_code_text(0xC000_013Au32 as i32, Os::Windows),
            "-1073741510 (0xC000013A: interrupted by Ctrl+C)"
        );
        assert_eq!(
            exit_code_text(0xC000_0005u32 as i32, Os::Windows),
            "-1073741819 (0xC0000005)"
        );
        assert_eq!(exit_code_text(-1, Os::Windows), "-1");
        assert_eq!(exit_code_text(130, Os::Windows), "130");
    }

    #[test]
    fn finished_message() {
        let status = ExitStatus {
            code: Some(3),
            signal: None,
        };
        assert_eq!(
            status.finished_message(),
            "Process finished with exit code 3"
        );
    }
}
