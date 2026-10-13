use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::RwLock;

/// The argument that makes a program act as the interrupt helper; the
/// process id to interrupt follows it as the last argument.
pub const INTERRUPT_HELPER_ARG: &str = "--pyoxide-send-ctrl-c";

/// The program that sends Ctrl+C on Windows (see
/// [`set_interrupt_helper`]): started as `<program> <args...>
/// --pyoxide-send-ctrl-c <pid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptHelper {
    /// The executable.
    pub program: PathBuf,
    /// Arguments before [`INTERRUPT_HELPER_ARG`].
    pub args: Vec<OsString>,
}

impl InterruptHelper {
    /// `program` without leading arguments.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        InterruptHelper {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// The running executable, which must call
    /// [`run_interrupt_helper_if_requested`] first thing in `main`.
    pub fn current_exe() -> Option<Self> {
        std::env::current_exe().ok().map(InterruptHelper::new)
    }
}

static HELPER: RwLock<Option<InterruptHelper>> = RwLock::new(None);

/// Sets the program [`ProcessHandle::interrupt`](crate::ProcessHandle::interrupt)
/// starts on Windows to send Ctrl+C, as the reference does with WinP's
/// `sendctrlc` helper: a console Ctrl+C event reaches every process attached
/// to the console it is sent to, so it is sent from a separate process that
/// attaches to the child's console, never from the IDE itself. The program
/// must call [`run_interrupt_helper_if_requested`] first thing in `main`
/// (the application sets itself, [`InterruptHelper::current_exe`]). Without a
/// helper an interrupt fails on Windows and a stop kills. Unused elsewhere.
pub fn set_interrupt_helper(helper: Option<InterruptHelper>) {
    *HELPER.write().unwrap_or_else(|e| e.into_inner()) = helper;
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn interrupt_helper() -> Option<InterruptHelper> {
    HELPER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Call first thing in `main` of a program used as the interrupt helper. On
/// Windows, when the arguments end with `--pyoxide-send-ctrl-c <pid>`, sends
/// Ctrl+C to the console of process `<pid>` (leaving its own console,
/// attaching to the target's, ignoring Ctrl+C itself, generating the event)
/// and exits: with 0, or with the Windows error code of the step that
/// failed. Otherwise, and on other systems, returns at once.
pub fn run_interrupt_helper_if_requested() {
    #[cfg(windows)]
    if let Some(pid) = requested_pid(std::env::args_os().skip(1).collect()) {
        std::process::exit(crate::sys::send_ctrl_c(pid));
    }
}

/// The process id when `args` (without the program) end with
/// [`INTERRUPT_HELPER_ARG`] and a number.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn requested_pid(args: Vec<OsString>) -> Option<u32> {
    let [.., flag, pid] = args.as_slice() else {
        return None;
    };
    if flag != INTERRUPT_HELPER_ARG {
        return None;
    }
    pid.to_str()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_helper_is_requested_by_the_last_two_arguments() {
        assert_eq!(
            requested_pid(args(&["--pyoxide-send-ctrl-c", "1234"])),
            Some(1234)
        );
        assert_eq!(
            requested_pid(args(&["sub", "--pyoxide-send-ctrl-c", "7"])),
            Some(7)
        );
        assert_eq!(requested_pid(args(&[])), None);
        assert_eq!(requested_pid(args(&["project"])), None);
        assert_eq!(requested_pid(args(&["--pyoxide-send-ctrl-c", "x"])), None);
        assert_eq!(requested_pid(args(&["--pyoxide-send-ctrl-c"])), None);
        assert_eq!(requested_pid(args(&["a", "b"])), None);
    }
}
