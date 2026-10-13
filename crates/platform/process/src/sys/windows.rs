use std::collections::HashSet;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, FILETIME, HANDLE, TRUE};
use windows_sys::Win32::System::Console::{
    AttachConsole, CTRL_BREAK_EVENT, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler,
    SetStdHandle,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE, TerminateProcess,
};

use crate::ExitStatus;

/// The exit code a killed process gets: `(UINT)-1`, as WinP's
/// `TerminateProcess(hProcess, -1)`.
const KILLED_EXIT_CODE: u32 = u32::MAX;

/// Serialises spawning with sending Ctrl+C: while the IDE is attached to a
/// child's console and ignores Ctrl+C, a new child would inherit the
/// "ignore Ctrl+C" flag. One console at a time.
static CONSOLE_LOCK: Mutex<()> = Mutex::new(());

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
    let _console = CONSOLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

/// Sends a console Ctrl+C event to the process's console, as the reference
/// does through WinP's `sendctrlc` helper (`FreeConsole`, `AttachConsole`,
/// `SetConsoleCtrlHandler(NULL, TRUE)`, `GenerateConsoleCtrlEvent(CTRL_C_EVENT,
/// 0)`), but inside the IDE's process: the IDE ignores the event itself and
/// detaches again. Fails, without touching anything, when the IDE is
/// attached to a console of its own (a console-subsystem build started from
/// a terminal): detaching from it could not be undone.
pub(crate) fn interrupt(process: &OsProcess) -> io::Result<()> {
    let _console = CONSOLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    install_ctrl_handler();
    // SAFETY: console and standard handle calls of the own process, under
    // the console lock; every state change is undone before returning.
    unsafe {
        let saved = [
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
            GetStdHandle(STD_ERROR_HANDLE),
        ];
        if AttachConsole(process.pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        SetConsoleCtrlHandler(None, TRUE);
        swallow_ctrl_events_for(Duration::from_secs(5));
        let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
        let err = io::Error::last_os_error();
        FreeConsole();
        SetStdHandle(STD_INPUT_HANDLE, saved[0]);
        SetStdHandle(STD_OUTPUT_HANDLE, saved[1]);
        SetStdHandle(STD_ERROR_HANDLE, saved[2]);
        // Children started from now on must not inherit "ignore Ctrl+C". The
        // event may reach this process later; the handler swallows it.
        SetConsoleCtrlHandler(None, FALSE);
        if sent == 0 { Err(err) } else { Ok(()) }
    }
}

/// Monotonic milliseconds until which Ctrl+C and Ctrl+Break reaching this
/// process are swallowed (they come from our own interrupts).
static SWALLOW_UNTIL: AtomicU64 = AtomicU64::new(0);

fn now_millis() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn swallow_ctrl_events_for(duration: Duration) {
    SWALLOW_UNTIL.store(
        now_millis() + duration.as_millis() as u64,
        Ordering::Release,
    );
}

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> i32 {
    let ours = ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT;
    if ours && now_millis() < SWALLOW_UNTIL.load(Ordering::Acquire) {
        TRUE
    } else {
        FALSE
    }
}

fn install_ctrl_handler() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        now_millis();
        // SAFETY: registers a handler routine that lives for the whole program.
        unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), TRUE) };
    });
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
    let exited = unsafe {
        windows_sys::Win32::System::Threading::GetExitCodeProcess(root, &mut code) != 0
            && code != windows_sys::Win32::Foundation::STILL_ACTIVE as u32
    };
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
        for &(pid, ppid) in &entries {
            if ppid == 0 || pid == parent || ppid != parent || seen.contains(&pid) {
                continue;
            }
            // SAFETY: a handle opened, used and closed here.
            let created = unsafe {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
                if h.is_null() {
                    None
                } else {
                    let t = creation_time(h);
                    CloseHandle(h);
                    t
                }
            };
            if created.is_some_and(|t| t >= root_created) {
                seen.insert(pid);
                found.push(pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found
}

/// (pid, parent pid) of every process.
fn snapshot() -> Vec<(u32, u32)> {
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
                out.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}
