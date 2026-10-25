use std::collections::HashSet;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, FALSE, FILETIME, GetLastError, HANDLE, STILL_ACTIVE, TRUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::ExitStatus;
use crate::helper::{INTERRUPT_HELPER_ARG, interrupt_helper};

/// The exit code a killed process gets: `(UINT)-1`, as WinP's
/// `TerminateProcess(hProcess, -1)`.
const KILLED_EXIT_CODE: u32 = u32::MAX;

/// How long the interrupt helper may take to start, attach and send.
const HELPER_TIMEOUT_MS: u32 = 10_000;

/// What interrupt and kill need: the id and an owned handle, which keeps the
/// process object (and so its id) from being reused while we hold it.
pub(crate) struct OsProcess {
    pid: u32,
    handle: OwnedHandle,
}

impl OsProcess {
    pub(crate) fn new(child: &Child) -> io::Result<Self> {
        Ok(OsProcess {
            pid: child.id(),
            handle: child.as_handle().try_clone_to_owned()?,
        })
    }
}

/// Starts the process with its own console without a window: a console
/// child of a GUI application gets no visible console window, and it can
/// still receive a console Ctrl+C event.
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    command.creation_flags(CREATE_NO_WINDOW);
    command.spawn()
}

/// A pipe handle as a number (`HANDLE` is a pointer and not `Send`).
pub(crate) type Pipe = usize;

pub(crate) fn pipe_of(pipe: &impl AsRawHandle) -> Pipe {
    pipe.as_raw_handle() as usize
}

/// Bytes waiting in a pipe, `None` when unknown.
pub(crate) fn pipe_available(pipe: Pipe) -> Option<usize> {
    let mut available: u32 = 0;
    // SAFETY: the handle is open while the reader that owns it has not
    // finished, which the caller checks under a lock; only the count is read.
    let ok = unsafe {
        PeekNamedPipe(
            pipe as HANDLE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available as usize)
}

pub(crate) fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: status.code(),
        signal: None,
    }
}

/// Sends a console Ctrl+C event to the process's console the way the
/// reference does, from a separate helper process (WinP's `sendctrlc`):
/// starts the [`interrupt_helper`] with [`INTERRUPT_HELPER_ARG`] and the
/// process id, without a window, and waits for its exit code. This process
/// never attaches to the child's console, so the event cannot reach it.
/// Fails when no helper is set, the helper cannot start, does not answer in
/// time, or reports an error (its exit code is the Windows error).
pub(crate) fn interrupt(process: &OsProcess) -> io::Result<()> {
    let helper = interrupt_helper().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "no interrupt helper is set (process::set_interrupt_helper)",
        )
    })?;
    let mut child = Command::new(&helper.program)
        .args(&helper.args)
        .arg(INTERRUPT_HELPER_ARG)
        .arg(process.pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    // SAFETY: waits on the helper's own process handle, which `child` keeps
    // open.
    let waited = unsafe { WaitForSingleObject(child.as_raw_handle() as HANDLE, HELPER_TIMEOUT_MS) };
    if waited != WAIT_OBJECT_0 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the interrupt helper did not answer",
        ));
    }
    match child.wait()?.code() {
        Some(0) => Ok(()),
        Some(code) => Err(io::Error::from_raw_os_error(code)),
        None => Err(io::Error::other("the interrupt helper failed")),
    }
}

/// The interrupt helper's work, in the helper process (WinP's
/// `sendctrlc/main.cpp`): leave the own console, attach to the target's,
/// ignore Ctrl+C here, and send Ctrl+C to every process of that console.
/// Returns the exit code: 0, or the Windows error of the failed call.
pub(crate) fn send_ctrl_c(pid: u32) -> i32 {
    // SAFETY: console calls of this short-lived helper process only.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return last_error_code();
        }
        SetConsoleCtrlHandler(None, TRUE);
        if GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) == 0 {
            return last_error_code();
        }
    }
    0
}

/// `GetLastError` as a non-zero exit code.
fn last_error_code() -> i32 {
    // SAFETY: reads the calling thread's last error.
    let code = unsafe { GetLastError() } as i32;
    if code == 0 { 1 } else { code }
}

/// Kills the process tree the way the reference's WinP does
/// (`KillProcessTreeWinHelper`): the descendants found through the parent
/// ids of a process snapshot, keeping only processes created at or after the
/// process (parent ids are not updated when a parent exits, so an id may have
/// been reused), killed in reverse order, then the process, all with exit
/// code -1.
pub(crate) fn kill(process: &OsProcess) -> io::Result<()> {
    let root = process.handle.as_raw_handle() as HANDLE;
    let tree = match creation_time(root) {
        Some(created) => descendants(process.pid, created),
        None => Vec::new(),
    };
    for &pid in tree.iter().rev() {
        // SAFETY: a handle opened, used and closed here.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, FALSE, pid);
            if !h.is_null() {
                TerminateProcess(h, KILLED_EXIT_CODE);
                CloseHandle(h);
            }
        }
    }
    // SAFETY: our own handle of the process.
    if unsafe { TerminateProcess(root, KILLED_EXIT_CODE) } != 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    // Access denied after the process exited: nothing left to kill.
    let mut code: u32 = 0;
    // SAFETY: reads the exit code through our own handle.
    let exited = unsafe { GetExitCodeProcess(root, &mut code) != 0 && code != STILL_ACTIVE as u32 };
    if exited { Ok(()) } else { Err(err) }
}

fn creation_time(handle: HANDLE) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: four out-parameters of the right type.
    let ok = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    (ok != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

/// Descendants of `root` created at or after `root_created`, in the order
/// found (breadth first).
fn descendants(root: u32, root_created: u64) -> Vec<u32> {
    let entries = snapshot();
    let mut found = vec![root];
    let mut seen: HashSet<u32> = HashSet::from([root]);
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        for entry in &entries {
            let (pid, ppid) = (entry.pid, entry.parent);
            if ppid == 0 || pid == parent || ppid != parent || seen.contains(&pid) {
                continue;
            }
            if creation_time_of(pid).is_some_and(|t| t >= root_created) {
                seen.insert(pid);
                found.push(pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found
}

/// The direct children of `pid` created at or after it (a parent id is not
/// updated when the parent exits, so an id may have been reused), without
/// the console hosts a pseudo-console starts.
pub(crate) fn child_pids(pid: u32) -> Vec<u32> {
    let Some(created) = creation_time_of(pid) else {
        return Vec::new();
    };
    snapshot()
        .into_iter()
        .filter(|e| e.parent == pid && e.pid != pid && !is_console_host(&e.exe))
        .filter(|e| creation_time_of(e.pid).is_some_and(|t| t >= created))
        .map(|e| e.pid)
        .collect()
}

/// The programs that host a console and are not commands of their own.
fn is_console_host(exe: &str) -> bool {
    exe.eq_ignore_ascii_case("conhost.exe") || exe.eq_ignore_ascii_case("OpenConsole.exe")
}

/// When the process was created (`FILETIME` ticks), `None` when it cannot
/// be opened (it ended, or access is denied).
fn creation_time_of(pid: u32) -> Option<u64> {
    // SAFETY: a handle opened, used and closed here.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if h.is_null() {
            return None;
        }
        let created = creation_time(h);
        CloseHandle(h);
        created
    }
}

/// A process of the snapshot.
struct Entry {
    pid: u32,
    parent: u32,
    /// The program's file name.
    exe: String,
}

/// Every process.
fn snapshot() -> Vec<Entry> {
    let mut out = Vec::new();
    // SAFETY: the snapshot handle is closed below; the entry has its size set.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let name = &entry.szExeFile;
                let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                out.push(Entry {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    exe: String::from_utf16_lossy(&name[..len]),
                });
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

/// A process with this id exists and has not exited (also one we may not
/// open).
pub(crate) fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: a handle opened, used and closed here.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if h.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code: u32 = 0;
        let alive = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        CloseHandle(h);
        alive
    }
}

#[cfg(test)]
mod tests {
    use super::is_console_host;

    #[test]
    fn console_hosts_are_not_commands() {
        assert!(is_console_host("conhost.exe"));
        assert!(is_console_host("OpenConsole.EXE"));
        assert!(!is_console_host("python.exe"));
    }
}
